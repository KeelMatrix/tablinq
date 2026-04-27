import { mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import type { Page } from "playwright-core";
import { resolveTablinqAssetPath } from "./assets";
import { chooseUniqueArtifactPath } from "./artifacts";
import { inferAuthenticated, safeTitle } from "./capabilityProbe";
export type {
  AskExecutionCommand,
  AskResultSnapshot,
  BrowserAdapterOptions,
  BrowserDiagnosticsSnapshot,
  BrowserLaunchConfig,
  BrowserOpenCommand,
  BrowserStatusSnapshot,
  RequestStateSnapshot,
  ResearchExecutionCommand,
  ResearchProgressSnapshot,
  ResearchResultSnapshot,
  StreamChunkSnapshot,
} from "./contracts";
import type {
  AskExecutionCommand,
  AskResultSnapshot,
  BrowserAdapterOptions,
  BrowserDiagnosticsSnapshot,
  BrowserOpenCommand,
  BrowserStatusSnapshot,
  DomNeighborhoodSnapshot,
  RequestStateSnapshot,
  ResearchExecutionCommand,
  ResearchProgressSnapshot,
  ResearchResultSnapshot,
  StreamChunkSnapshot,
} from "./contracts";
import {
  detectBrowserExecutable,
  resolveBrowserLaunchConfig,
  resolveExtensionDir,
} from "./launch";
import { BrowserExecutionController } from "./execution";
import type { AskBaseline } from "./snapshotState";
import { normalizeOptionalString } from "./snapshotState";
export { detectBrowserExecutable, resolveBrowserLaunchConfig } from "./launch";
export { deepResearchStartDelayMs } from "./timing";
export { interactionJitterMs } from "./timing";
export {
  MAX_UPLOAD_FILE_BYTES,
  adaptivePollIntervalMs,
  highRiskActionCooldownMs,
  noteProviderPressure,
  noteProviderPressureFromText,
  pacingRandomValueFromSeed,
  providerPressureCooldownMs,
  resetProviderPressureForTest,
  resetSessionRhythmForTest,
  uploadAckTimeoutMsForFiles,
} from "./timing";
import { conversationIdFromUrl, resolveConversationUrl } from "./urls";
export {
  conversationIdFromUrl,
  resolveConversationUrl,
  workspaceKeyFromUrl,
} from "./urls";
export { chooseUniqueArtifactPath } from "./artifacts";
import { CapabilityStrategyCache } from "./browserAdapter/capabilityCache";
import { CapabilityProbeController } from "./browserAdapter/capabilityProbeController";
import {
  captureBrowserDiagnostics,
  DiagnosticsBundleCapture,
} from "./browserAdapter/diagnostics";
import { BrowserPageRegistry } from "./browserAdapter/pageRegistry";
import { BrowserSessionController } from "./browserAdapter/sessionController";

type StatusSnapshotOptions = {
  recoverManagedPage: boolean;
  refreshCapabilities: boolean;
  clearCapabilitiesWhenStopped: boolean;
  includeTitle: boolean;
};

export class BrowserAdapter {
  private readonly profileDir: string;
  private readonly downloadsDir: string;
  private readonly defaultTargetUrl: string;
  private readonly defaultBrowserPath: string | null;
  private readonly connectionMode:
    | "attach_first"
    | "managed_fallback"
    | "managed_only";
  private readonly debugEndpoint: string | null;
  private readonly extensionRequired: boolean;
  private lastScreenshotPath: string | null = null;
  private readonly pageRegistry = new BrowserPageRegistry();
  private readonly capabilityCache: CapabilityStrategyCache;
  private readonly capabilities: CapabilityProbeController;
  private readonly diagnosticsCapture: DiagnosticsBundleCapture;
  private readonly session: BrowserSessionController;
  private readonly execution: BrowserExecutionController;

  constructor(options: BrowserAdapterOptions) {
    this.profileDir = resolve(options.profileDir);
    this.downloadsDir = resolve(options.downloadsDir);
    this.defaultTargetUrl = options.defaultTargetUrl;
    this.defaultBrowserPath = options.defaultBrowserPath ?? null;
    this.connectionMode = options.connectionMode ?? "managed_only";
    this.debugEndpoint = options.debugEndpoint ?? "http://127.0.0.1:9222";
    this.extensionRequired = options.extensionRequired ?? true;
    this.capabilityCache = new CapabilityStrategyCache(
      resolve(this.profileDir, "tablinq-capability-cache.json"),
      resolve(
        resolveTablinqAssetPath([
          "selector-packs",
          "v1",
          "chatgpt-default.json",
        ]),
      ),
    );
    this.capabilities = new CapabilityProbeController(
      this.pageRegistry,
      this.capabilityCache,
      () => this.session.activePageId,
      () => this.session.attachedOverCdp,
    );
    this.diagnosticsCapture = new DiagnosticsBundleCapture(this.profileDir);
    this.session = new BrowserSessionController(
      {
        profileDir: this.profileDir,
        downloadsDir: this.downloadsDir,
        defaultTargetUrl: this.defaultTargetUrl,
        defaultBrowserPath: this.defaultBrowserPath,
        extensionDir: resolveExtensionDir(options.extensionDir),
        connectionMode: this.connectionMode,
        debugEndpoint: this.debugEndpoint,
        autoOpenTargetTab: options.autoOpenTargetTab ?? true,
      },
      this.pageRegistry,
      {
        refreshCapabilities: (page) => this.capabilities.refresh(page),
        applyPageStateToActiveTarget: (state) =>
          this.capabilities.applyPageStateToActiveTarget(state),
        clearActiveCapabilityState: () => this.capabilities.clearActiveState(),
      },
    );
    this.execution = new BrowserExecutionController({
      getCurrentTargetUrl: () => this.session.targetUrl,
      setCurrentTargetUrl: (url) => {
        this.session.targetUrl = url;
      },
      getLastCapabilityReport: () =>
        this.capabilities.snapshot().lastCapabilityReport,
      getDownloadsDir: () => this.downloadsDir,
      refreshCapabilities: async (page) => {
        await this.refreshCapabilities(page);
      },
      openExecutionPage: (targetUrl) => this.openExecutionPage(targetUrl),
      ensureManagedPage: (targetUrl) => this.ensureManagedPage(targetUrl),
      resolveAskTarget: (conversationId, newConversation) =>
        this.resolveAskTarget(conversationId, newConversation),
      resolveConversationTarget: (conversationId) =>
        this.resolveConversationTarget(conversationId),
      isManagedPage: (page) => this.session.isManagedPage(page),
      activateRecoveredPage: (page) => this.activateRecoveredPage(page),
    });
  }

  async open(command: BrowserOpenCommand): Promise<BrowserStatusSnapshot> {
    const targetUrl = command.targetUrl || this.defaultTargetUrl;
    const browserPath = command.browserPath ?? this.defaultBrowserPath;
    const headless = command.headless ?? false;
    const forceReload = command.forceReload ?? false;

    this.session.configureOpen(targetUrl, browserPath ?? null);

    this.capabilityCache.hydrate();
    await this.session.ensureContext(headless, browserPath ?? undefined);

    const page = await this.ensureManagedPage(targetUrl, forceReload);
    await this.refreshCapabilities(page);
    return this.health();
  }

  async health(): Promise<BrowserStatusSnapshot> {
    return this.buildStatusSnapshot({
      recoverManagedPage: true,
      refreshCapabilities: true,
      clearCapabilitiesWhenStopped: true,
      includeTitle: true,
    });
  }

  async snapshot(): Promise<BrowserStatusSnapshot> {
    return this.buildStatusSnapshot({
      recoverManagedPage: false,
      refreshCapabilities: false,
      clearCapabilitiesWhenStopped: false,
      includeTitle: false,
    });
  }

  private async buildStatusSnapshot(
    options: StatusSnapshotOptions,
  ): Promise<BrowserStatusSnapshot> {
    if (options.recoverManagedPage) {
      await this.session.recoverManagedPage();
    }
    const page = this.session.activePage;
    const running = Boolean(page && !page.isClosed());

    if (running && options.refreshCapabilities) {
      await this.refreshCapabilities(page!);
    } else if (!running && options.clearCapabilitiesWhenStopped) {
      this.capabilities.clearActiveState();
    }

    const capability = this.capabilities.snapshot();
    return {
      running,
      browser_kind: this.session.statusBrowserKind(),
      profile_dir: this.profileDir,
      downloads_dir: this.downloadsDir,
      target_url: this.session.targetUrl,
      current_url: running ? page!.url() : null,
      page_title:
        running && options.includeTitle ? await safeTitle(page!) : null,
      page_ready: running,
      authenticated: running ? inferAuthenticated(page!.url()) : false,
      reload_count: this.session.activeReloadCount,
      screenshot_path: this.lastScreenshotPath,
      browser_path: this.session.browserPath,
      extension_connected: capability.extensionConnected,
      capability_report: capability.lastCapabilityReport,
      attach_mode: this.connectionMode,
      browser_owner: this.session.activeBrowserOwner,
      cdp_attached: this.session.attachedOverCdp,
      target_tab_found: running,
      target_tab_id: running ? this.session.activePageId : null,
      target_workspace_key: running ? this.session.activeWorkspaceKey() : null,
      debug_endpoint: this.debugEndpoint,
      auth_source:
        running && inferAuthenticated(page!.url()) ? "page" : "unknown",
      extension_required: this.extensionRequired,
      last_capability_report_at: running
        ? this.capabilities.activeReportedAt()
        : null,
    };
  }

  async screenshot(path: string): Promise<BrowserStatusSnapshot> {
    await this.session.recoverManagedPage();
    const page = this.session.activePage;
    if (!page || page.isClosed()) {
      throw new Error("target page is not open");
    }

    const screenshotPath = resolve(path);
    await mkdir(dirname(screenshotPath), { recursive: true });
    await page.screenshot({ fullPage: true, path: screenshotPath });
    this.lastScreenshotPath = screenshotPath;
    return this.health();
  }

  async close(): Promise<BrowserStatusSnapshot> {
    await this.session.close();
    return this.health();
  }

  async shutdown(): Promise<BrowserStatusSnapshot> {
    return this.close();
  }

  getManagedPageForTest(): Page | undefined {
    return this.session.activePage;
  }

  getLastDomNeighborhoodsForTest(): DomNeighborhoodSnapshot[] {
    return this.capabilities.snapshot().lastDomNeighborhoods;
  }

  async diagnostics(bundleDir?: string): Promise<BrowserDiagnosticsSnapshot> {
    const page = this.session.activePage?.isClosed()
      ? undefined
      : this.session.activePage;
    if (page) {
      await this.refreshCapabilities(page);
    }
    const capability = this.capabilities.snapshot();
    return captureBrowserDiagnostics({
      capabilityCache: this.capabilityCache,
      diagnosticsCapture: this.diagnosticsCapture,
      page,
      bundleDir,
      currentBrowserPath: this.session.browserPath,
      lastCapabilityReport: capability.lastCapabilityReport,
      lastUiSignature: capability.lastUiSignature,
      lastDomNeighborhoods: capability.lastDomNeighborhoods,
      onScreenshotCaptured: (path) => {
        this.lastScreenshotPath = path;
      },
    });
  }

  async ask(
    command: AskExecutionCommand,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<AskResultSnapshot> {
    return this.execution.ask(command, onChunk, onState);
  }

  async research(
    command: ResearchExecutionCommand,
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<ResearchResultSnapshot> {
    return this.execution.research(command, onUpdate, onState);
  }

  async recoverAsk(
    command: AskExecutionCommand,
    state: {
      emittedText: string;
      nextSeq: number;
      knownConversationId?: string | null;
      knownMessageId?: string | null;
      askBaseline?: AskBaseline | null;
    },
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<AskResultSnapshot> {
    return this.execution.recoverAsk(command, state, onChunk, onState);
  }

  async recoverResearch(
    command: ResearchExecutionCommand,
    state: { emittedText: string; nextSeq: number },
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<ResearchResultSnapshot> {
    return this.execution.recoverResearch(command, state, onUpdate, onState);
  }

  private async openExecutionPage(targetUrl: string): Promise<Page> {
    await this.session.ensureContext(
      this.session.headless,
      this.session.browserPath ?? undefined,
    );
    const page = await this.ensureManagedPage(targetUrl);
    if (!inferAuthenticated(page.url())) {
      throw new Error(
        "auth required: target browser session is not authenticated",
      );
    }
    return page;
  }

  private activateRecoveredPage(page: Page): void {
    this.session.activateRecoveredPage(page);
  }

  private async reconcileAskAfterInterruption(
    ...args: Parameters<
      BrowserExecutionController["reconcileAskAfterInterruptionForTest"]
    >
  ): ReturnType<
    BrowserExecutionController["reconcileAskAfterInterruptionForTest"]
  > {
    return this.execution.reconcileAskAfterInterruptionForTest(...args);
  }

  private async ensureManagedPage(
    targetUrl: string,
    forceReload = false,
  ): Promise<Page> {
    return this.session.ensureManagedPage(targetUrl, forceReload);
  }

  private resolveConversationTarget(conversationId?: string | null): string {
    return resolveConversationUrl(this.session.targetUrl, conversationId);
  }

  private resolveAskTarget(
    conversationId?: string | null,
    newConversation = false,
  ): string {
    if (newConversation) {
      return this.resolveConversationTarget(null);
    }
    const normalizedConversationId = normalizeOptionalString(conversationId);
    if (normalizedConversationId) {
      return this.resolveConversationTarget(normalizedConversationId);
    }
    return conversationIdFromUrl(this.session.targetUrl)
      ? this.session.targetUrl
      : this.resolveConversationTarget(null);
  }

  private async refreshCapabilities(page: Page) {
    return this.capabilities.refresh(page);
  }
}
