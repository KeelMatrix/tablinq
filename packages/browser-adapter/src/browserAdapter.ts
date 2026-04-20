import { readFileSync, writeFileSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import {
  type Browser,
  chromium,
  type BrowserContext,
  type Page,
} from "playwright-core";
import {
  loadProbeSource,
  resolveTablinqAssetPath,
} from "./assets";
import { chooseUniqueArtifactPath } from "./artifacts";
import {
  captureAriaSnapshot,
  captureDomSnapshot,
  hasUsableCapabilityReport,
  inferAuthenticated,
  logRecoveryDecision,
  safePageUrl,
  safeTitle,
  telemetryKey,
} from "./capabilityProbe";
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
  BrowserLaunchConfig,
  BrowserOpenCommand,
  BrowserStatusSnapshot,
  CapabilityBridgeMessage,
  CapabilityKey,
  CapabilityMatrixSnapshot,
  CapabilityReportPayload,
  DomNeighborhoodSnapshot,
  ProbeDocumentOptions,
  SelectorPackSnapshot,
  StrategyAttemptSnapshot,
  StrategyTelemetrySnapshot,
  RequestStateSnapshot,
  ResearchExecutionCommand,
  ResearchProgressSnapshot,
  ResearchResultSnapshot,
  StreamChunkSnapshot,
} from "./contracts";
import {
  detectBrowserAvailability,
  detectBrowserExecutable,
  inferBrowserKind,
  resolveBrowserLaunchConfig,
  resolveExtensionDir,
} from "./launch";
import { BrowserExecutionController } from "./execution";
import {
  applyAskConfiguration,
  beginDeepResearchRun,
  clickSubmit,
  collectArtifacts,
  ensureResearchComposerFilled,
  fillComposer,
  isTransientPageError,
  readAskBaseline,
  readAskSnapshot,
  readLiveResearchSnapshot,
  readResearchSubmissionState,
  resetConversationIdentity,
  sleep,
  submitResearch,
  uploadAttachments,
  waitForSubmitReadyAfterUploads,
} from "./pageRuntime";
import {
  AskBaseline,
  AskSnapshot,
  DeepResearchFrameSnapshot,
  ResearchSnapshot,
  ResearchSubmissionStateSnapshot,
  canFinalizeRecoveredAskSnapshot,
  canFinalizeRecoveredAskText,
  computeStallTimeoutMs,
  isTransientAskStatusText,
  markdownCompatibleRecoveredText,
  normalizeComparableAskText,
  normalizeOptionalString,
  normalizeRecoveredAskText,
  normalizeResearchSnapshot,
  placeholderCompatibleText,
  researchSubmissionStateToSnapshot,
  sameAskCompletionSnapshot,
  sameResearchCompletionSnapshot,
  textRelation,
} from "./snapshotState";
export { detectBrowserExecutable, resolveBrowserLaunchConfig } from "./launch";
import {
  ASK_STALL_RECOVERY_GRACE_MS,
  ASK_TIMEOUT_MS,
  ATTACH_PROBE_GRACE_MS,
  DEEP_RESEARCH_HANDOFF_TIMEOUT_MS,
  INTERACTION_TIMEOUT_MS,
  MAX_RECOVERY_ATTEMPTS,
  NAVIGATION_TIMEOUT_MS,
  PROBE_TIMEOUT_MS,
  RESEARCH_SUBMISSION_START_TIMEOUT_MS,
  RESEARCH_TIMEOUT_MS,
  SNAPSHOT_POLL_INTERVAL_MS,
  STABLE_DONE_POLLS,
  SUBMIT_READY_AFTER_UPLOAD_TIMEOUT_MS,
  deepResearchStartDelayMs,
} from "./timing";
export { deepResearchStartDelayMs } from "./timing";
export { MAX_UPLOAD_FILE_BYTES, uploadAckTimeoutMsForFiles } from "./timing";
import {
  buildTargetPageId,
  conversationIdFromUrl,
  matchesTargetLocation,
  matchesTargetWorkspace,
  researchRecoveryLocationScore,
  resolveConversationUrl,
  uniquePages,
  workspaceKeyFromUrl,
} from "./urls";
export {
  conversationIdFromUrl,
  resolveConversationUrl,
  workspaceKeyFromUrl,
} from "./urls";
export { chooseUniqueArtifactPath } from "./artifacts";

interface CapabilityCacheEntry {
  workspace_key: string;
  saved_at: string;
  ui_signature: string | null;
  preferred_strategies: Partial<Record<CapabilityKey, string>>;
  report: CapabilityReportPayload;
}

interface StrategyTelemetryStat {
  capability: CapabilityKey;
  strategy_id: string;
  success_count: number;
  failure_count: number;
  last_used_at: string | null;
}

interface CapabilityCacheFile {
  version: 2;
  workspaces: Record<string, CapabilityCacheEntry>;
  telemetry: StrategyTelemetryStat[];
}

interface TargetPageState {
  page_id: string;
  last_seen_url: string | null;
  workspace_key: string | null;
  extension_connected: boolean;
  capability_report: CapabilityReportPayload | null;
  ui_signature: string | null;
  dom_neighborhoods: DomNeighborhoodSnapshot[];
  last_capability_report_at: string | null;
  probe_ready_until_ms: number;
}

function isRecoverableTargetPageError(error: unknown): boolean {
  if (isTransientPageError(error)) {
    return true;
  }
  const message = error instanceof Error ? error.message : String(error);
  return (
    message.includes("Timeout") ||
    message.includes("timed out") ||
    message.includes("Target closed")
  );
}

export class BrowserAdapter {
  private browser: Browser | undefined;
  private context: BrowserContext | undefined;
  private managedPage: Page | undefined;
  private readonly profileDir: string;
  private readonly downloadsDir: string;
  private readonly defaultTargetUrl: string;
  private readonly defaultBrowserPath: string | null;
  private readonly extensionDir: string | null;
  private readonly connectionMode:
    | "attach_first"
    | "managed_fallback"
    | "managed_only";
  private readonly debugEndpoint: string | null;
  private readonly extensionRequired: boolean;
  private readonly autoOpenTargetTab: boolean;
  private currentTargetUrl: string;
  private currentBrowserPath: string | null;
  private currentHeadless = false;
  private reloadCount = 0;
  private lastScreenshotPath: string | null = null;
  private targetPageId: string | null = null;
  private targetPageCreatedByAdapter = false;
  private browserOwner: "external" | "managed" = "managed";
  private cdpAttached = false;
  private readonly observedPages = new WeakSet<Page>();
  private readonly observedContexts = new WeakSet<BrowserContext>();
  private readonly pageStates = new Map<string, TargetPageState>();
  private extensionConnected = false;
  private lastCapabilityReport: CapabilityReportPayload | null = null;
  private lastUiSignature: string | null = null;
  private readonly strategyTelemetry = new Map<
    string,
    Partial<Record<CapabilityKey, string>>
  >();
  private readonly strategyTelemetryStats = new Map<
    string,
    StrategyTelemetryStat
  >();
  private lastDomNeighborhoods: DomNeighborhoodSnapshot[] = [];
  private readonly capabilityCachePath: string;
  private capabilityCacheLoaded = false;
  private capabilityCache: CapabilityCacheFile = {
    version: 2,
    workspaces: {},
    telemetry: [],
  };
  private readonly selectorPackPath: string;
  private selectorPack: SelectorPackSnapshot | null = null;
  private readonly execution: BrowserExecutionController;

  constructor(options: BrowserAdapterOptions) {
    this.profileDir = resolve(options.profileDir);
    this.downloadsDir = resolve(options.downloadsDir);
    this.defaultTargetUrl = options.defaultTargetUrl;
    this.defaultBrowserPath = options.defaultBrowserPath ?? null;
    this.extensionDir = resolveExtensionDir(options.extensionDir);
    this.connectionMode = options.connectionMode ?? "managed_only";
    this.debugEndpoint = options.debugEndpoint ?? "http://127.0.0.1:9222";
    this.extensionRequired = options.extensionRequired ?? true;
    this.autoOpenTargetTab = options.autoOpenTargetTab ?? true;
    this.currentTargetUrl = options.defaultTargetUrl;
    this.currentBrowserPath = options.defaultBrowserPath ?? null;
    this.capabilityCachePath = resolve(
      this.profileDir,
      "tablinq-capability-cache.json",
    );
    this.selectorPackPath = resolve(
      resolveTablinqAssetPath(["selector-packs", "v1", "chatgpt-default.json"]),
    );
    this.execution = new BrowserExecutionController({
      getCurrentTargetUrl: () => this.currentTargetUrl,
      setCurrentTargetUrl: (url) => {
        this.currentTargetUrl = url;
      },
      getLastCapabilityReport: () => this.lastCapabilityReport,
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
      isManagedPage: (page) => page === this.managedPage,
      activateRecoveredPage: (page) => this.activateRecoveredPage(page),
    });
  }

  async open(command: BrowserOpenCommand): Promise<BrowserStatusSnapshot> {
    const targetUrl = command.targetUrl || this.defaultTargetUrl;
    const browserPath = command.browserPath ?? this.defaultBrowserPath;
    const headless = command.headless ?? false;

    this.currentTargetUrl = targetUrl;
    this.currentBrowserPath = browserPath ?? null;

    await this.ensureContext(headless, browserPath ?? undefined);

    const page = await this.ensureManagedPage(targetUrl);
    await this.refreshCapabilities(page);
    return this.health();
  }

  async health(): Promise<BrowserStatusSnapshot> {
    await this.recoverManagedPage();
    const page = this.managedPage;
    const running = Boolean(this.context && page && !page.isClosed());

    if (running) {
      await this.refreshCapabilities(page!);
    } else {
      this.clearActiveCapabilityState();
    }

    return {
      running,
      browser_kind: this.statusBrowserKind(),
      profile_dir: this.profileDir,
      downloads_dir: this.downloadsDir,
      target_url: this.currentTargetUrl,
      current_url: running ? page!.url() : null,
      page_title: running ? await safeTitle(page!) : null,
      page_ready: running ? !page!.isClosed() : false,
      authenticated: running ? inferAuthenticated(page!.url()) : false,
      reload_count: this.reloadCount,
      screenshot_path: this.lastScreenshotPath,
      browser_path: this.currentBrowserPath,
      extension_connected: this.extensionConnected,
      capability_report: this.lastCapabilityReport,
      attach_mode: this.connectionMode,
      browser_owner: this.browserOwner,
      cdp_attached: this.cdpAttached,
      target_tab_found: running,
      target_tab_id: running ? this.targetPageId : null,
      target_workspace_key: running ? this.activeWorkspaceKey() : null,
      debug_endpoint: this.debugEndpoint,
      auth_source:
        running && inferAuthenticated(page!.url()) ? "page" : "unknown",
      extension_required: this.extensionRequired,
      last_capability_report_at: running
        ? this.activeCapabilityReportedAt()
        : null,
    };
  }

  async snapshot(): Promise<BrowserStatusSnapshot> {
    const page = this.managedPage;
    const running = Boolean(this.context && page && !page.isClosed());

    return {
      running,
      browser_kind: this.statusBrowserKind(),
      profile_dir: this.profileDir,
      downloads_dir: this.downloadsDir,
      target_url: this.currentTargetUrl,
      current_url: running ? page!.url() : null,
      page_title: null,
      page_ready: running,
      authenticated: running ? inferAuthenticated(page!.url()) : false,
      reload_count: this.reloadCount,
      screenshot_path: this.lastScreenshotPath,
      browser_path: this.currentBrowserPath,
      extension_connected: this.extensionConnected,
      capability_report: this.lastCapabilityReport,
      attach_mode: this.connectionMode,
      browser_owner: this.browserOwner,
      cdp_attached: this.cdpAttached,
      target_tab_found: running,
      target_tab_id: running ? this.targetPageId : null,
      target_workspace_key: running ? this.activeWorkspaceKey() : null,
      debug_endpoint: this.debugEndpoint,
      auth_source:
        running && inferAuthenticated(page!.url()) ? "page" : "unknown",
      extension_required: this.extensionRequired,
      last_capability_report_at: running
        ? this.activeCapabilityReportedAt()
        : null,
    };
  }

  async screenshot(path: string): Promise<BrowserStatusSnapshot> {
    await this.recoverManagedPage();
    if (!this.managedPage || this.managedPage.isClosed()) {
      throw new Error("target page is not open");
    }

    const screenshotPath = resolve(path);
    await mkdir(dirname(screenshotPath), { recursive: true });
    await this.managedPage.screenshot({ fullPage: true, path: screenshotPath });
    this.lastScreenshotPath = screenshotPath;
    return this.health();
  }

  async close(): Promise<BrowserStatusSnapshot> {
    if (this.cdpAttached) {
      if (
        this.targetPageCreatedByAdapter &&
        this.managedPage &&
        !this.managedPage.isClosed()
      ) {
        await this.closeAttachedTargetPage(this.managedPage);
      }
      if (this.browser) {
        await this.browser.close().catch(() => undefined);
      }
    } else if (this.context) {
      await this.context.close();
    }
    this.browser = undefined;
    this.context = undefined;
    this.managedPage = undefined;
    this.targetPageId = null;
    this.targetPageCreatedByAdapter = false;
    this.browserOwner =
      this.connectionMode === "managed_only" ? "managed" : "external";
    this.cdpAttached = false;
    this.pageStates.clear();
    this.clearActiveCapabilityState();
    return this.health();
  }

  async shutdown(): Promise<BrowserStatusSnapshot> {
    return this.close();
  }

  getManagedPageForTest(): Page | undefined {
    return this.managedPage;
  }

  private statusBrowserKind(): string {
    if (this.currentBrowserPath) {
      return inferBrowserKind(this.currentBrowserPath);
    }
    return process.platform === "win32" ? "msedge" : "chromium";
  }

  private activeWorkspaceKey(): string | null {
    return this.activePageState()?.workspace_key ?? null;
  }

  private activeCapabilityReportedAt(): string | null {
    return this.activePageState()?.last_capability_report_at ?? null;
  }

  private activePageState(): TargetPageState | undefined {
    return this.targetPageId
      ? this.pageStates.get(this.targetPageId)
      : undefined;
  }

  getLastDomNeighborhoodsForTest(): DomNeighborhoodSnapshot[] {
    return [...this.lastDomNeighborhoods];
  }

  private clearActiveCapabilityState(): void {
    this.extensionConnected = false;
    this.lastCapabilityReport = null;
    this.lastDomNeighborhoods = [];
    this.lastUiSignature = null;
  }

  private ensurePageState(page: Page): TargetPageState {
    const pageId = buildTargetPageId(page);
    const existing = this.pageStates.get(pageId);
    if (existing) {
      return existing;
    }

    const state: TargetPageState = {
      page_id: pageId,
      last_seen_url: safePageUrl(page),
      workspace_key: safePageUrl(page)
        ? workspaceKeyFromUrl(safePageUrl(page)!)
        : null,
      extension_connected: false,
      capability_report: null,
      ui_signature: null,
      dom_neighborhoods: [],
      last_capability_report_at: null,
      probe_ready_until_ms: this.cdpAttached
        ? Date.now() + ATTACH_PROBE_GRACE_MS
        : 0,
    };
    this.pageStates.set(pageId, state);
    return state;
  }

  private updatePageLocationState(page: Page): TargetPageState {
    const state = this.ensurePageState(page);
    const currentUrl = safePageUrl(page);
    state.last_seen_url = currentUrl;
    state.workspace_key = currentUrl ? workspaceKeyFromUrl(currentUrl) : null;
    if (this.cdpAttached) {
      state.probe_ready_until_ms = Math.max(
        state.probe_ready_until_ms,
        Date.now() + ATTACH_PROBE_GRACE_MS,
      );
    }
    return state;
  }

  private recordCapabilitySuccess(
    page: Page,
    message: CapabilityBridgeMessage,
  ): TargetPageState {
    const state = this.updatePageLocationState(page);
    state.extension_connected = true;
    state.capability_report = message.report;
    state.ui_signature = message.runtime.ui_signature;
    state.dom_neighborhoods = message.runtime.dom_neighborhoods;
    state.last_capability_report_at = message.report.reported_at;
    this.strategyTelemetry.set(
      message.runtime.ui_signature,
      message.runtime.strategy_matches,
    );
    this.recordStrategyStats(message.runtime.strategy_matches);
    this.persistCapabilitySnapshot(page.url(), message);
    if (this.targetPageId === state.page_id) {
      this.applyPageStateToActiveTarget(state);
    }
    return state;
  }

  private recordCapabilityFailure(page: Page): TargetPageState {
    const state = this.updatePageLocationState(page);
    state.extension_connected = false;
    state.capability_report = null;
    state.ui_signature = null;
    state.dom_neighborhoods = [];
    state.last_capability_report_at = null;
    if (this.targetPageId === state.page_id) {
      this.applyPageStateToActiveTarget(state);
    }
    return state;
  }

  private applyPageStateToActiveTarget(
    state: TargetPageState | undefined,
  ): void {
    if (!state) {
      this.clearActiveCapabilityState();
      return;
    }
    this.extensionConnected = state.extension_connected;
    this.lastCapabilityReport = state.capability_report;
    this.lastUiSignature = state.ui_signature;
    this.lastDomNeighborhoods = [...state.dom_neighborhoods];
  }

  private activateTargetPage(page: Page, createdByAdapter: boolean): void {
    const state = this.updatePageLocationState(page);
    this.managedPage = page;
    this.targetPageId = state.page_id;
    this.targetPageCreatedByAdapter = createdByAdapter;
    this.applyPageStateToActiveTarget(state);
  }

  async diagnostics(bundleDir?: string): Promise<BrowserDiagnosticsSnapshot> {
    this.hydrateCapabilityCache();
    const selectorPack = this.ensureSelectorPackLoaded();
    let profileAccessError: string | null = null;
    let profileAccessible = true;
    try {
      await mkdir(this.profileDir, { recursive: true });
      writeFileSync(
        resolve(this.profileDir, ".tablinq-profile-check"),
        "ok",
        "utf8",
      );
    } catch (error) {
      profileAccessible = false;
      profileAccessError =
        error instanceof Error ? error.message : String(error);
    }

    const page =
      this.managedPage && !this.managedPage.isClosed()
        ? this.managedPage
        : undefined;
    if (page) {
      await this.refreshCapabilities(page);
    }

    const screenshotPath =
      bundleDir && page
        ? await this.captureBundleScreenshot(page, bundleDir).catch(() => null)
        : null;

    return {
      current_url: page?.url() ?? null,
      capability_matrix: this.lastCapabilityReport?.matrix ?? {
        composer_ready: false,
        submit_available: false,
        model_picker_visible: false,
        thinking_effort_visible: false,
        search_mode_visible: false,
        research_mode_visible: false,
        upload_control_visible: false,
        downloadable_artifacts_observable: false,
      },
      strategy_attempts: this.buildStrategyAttempts(),
      dom_snapshot: page
        ? await captureDomSnapshot(page, this.lastDomNeighborhoods)
        : "",
      aria_snapshot: page ? await captureAriaSnapshot(page) : "",
      screenshot_path: screenshotPath,
      selector_pack: selectorPack
        ? {
            pack_id: selectorPack.pack_id,
            version: `v${selectorPack.format_version}`,
            updated_at: selectorPack.updated_at ?? null,
            path: this.selectorPackPath,
          }
        : null,
      strategy_telemetry: this.getStrategyTelemetrySnapshots(),
      browser_detected: detectBrowserAvailability(
        process.platform,
        this.currentBrowserPath ?? undefined,
      ).detected,
      detected_browser_path: detectBrowserAvailability(
        process.platform,
        this.currentBrowserPath ?? undefined,
      ).path,
      profile_accessible: profileAccessible,
      profile_access_error: profileAccessError,
    };
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
    state: { emittedText: string; nextSeq: number },
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
    await this.ensureContext(
      this.currentHeadless,
      this.currentBrowserPath ?? undefined,
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
    this.context = page.context();
    this.observeContext(this.context);
    this.attachPage(page, true);
    this.targetPageCreatedByAdapter = false;
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

  private async ensureManagedPage(targetUrl: string): Promise<Page> {
    await this.recoverManagedPage();
    let page = this.managedPage;

    if (this.cdpAttached) {
      const preferred = await this.findAttachedTargetPage(targetUrl);
      if (preferred && preferred !== page) {
        this.context = preferred.context();
        this.observeContext(this.context);
        this.attachPage(preferred, true);
        this.targetPageCreatedByAdapter = false;
        page = preferred;
      }
    }

    if (!page || page.isClosed()) {
      page = await this.createTargetPage(targetUrl);
    }

    try {
      await this.navigateTargetPage(page, targetUrl);
    } catch (error) {
      if (!isRecoverableTargetPageError(error)) {
        throw error;
      }

      await this.discardAttachedTargetPage(page);
      page = await this.createTargetPage(targetUrl);
      await this.navigateTargetPage(page, targetUrl);
    }

    if (this.cdpAttached) {
      await this.cleanupStaleAttachedTargetPages(targetUrl, page);
    }

    return page;
  }

  private async ensureContext(
    headless: boolean,
    browserPath?: string,
  ): Promise<void> {
    if (this.context) {
      return;
    }

    this.currentHeadless = headless;
    this.hydrateCapabilityCache();
    await mkdir(this.profileDir, { recursive: true });
    await mkdir(this.downloadsDir, { recursive: true });
    const shouldTryAttach =
      this.connectionMode === "attach_first" ||
      this.connectionMode === "managed_fallback";
    let attachError: Error | null = null;

    if (shouldTryAttach) {
      try {
        await this.attachToExistingBrowser();
        return;
      } catch (error) {
        attachError = error instanceof Error ? error : new Error(String(error));
        if (this.connectionMode === "attach_first") {
          throw attachError;
        }
      }
    }

    try {
      await this.launchManagedContext(headless, browserPath);
    } catch (error) {
      if (attachError) {
        throw new Error(
          `${attachError.message}; managed fallback failed: ${error instanceof Error ? error.message : String(error)}`,
        );
      }
      throw error;
    }
  }

  private resolveConversationTarget(conversationId?: string | null): string {
    return resolveConversationUrl(this.currentTargetUrl, conversationId);
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
    return conversationIdFromUrl(this.currentTargetUrl)
      ? this.currentTargetUrl
      : this.resolveConversationTarget(null);
  }

  private async recoverManagedPage(): Promise<void> {
    if (!this.context) {
      return;
    }

    if (this.managedPage && !this.managedPage.isClosed()) {
      if (this.cdpAttached) {
        const preferred = await this.findAttachedTargetPage(
          this.currentTargetUrl,
        );
        if (preferred && preferred !== this.managedPage) {
          this.context = preferred.context();
          this.observeContext(this.context);
          this.attachPage(preferred, true);
          this.targetPageCreatedByAdapter = false;
          return;
        }
      }
      return;
    }

    const existing = this.cdpAttached
      ? await this.findAttachedTargetPage(this.currentTargetUrl)
      : this.context.pages().find((page) => !page.isClosed());
    if (existing) {
      this.context = existing.context();
      this.observeContext(this.context);
      this.attachPage(existing, true);
      this.targetPageCreatedByAdapter = false;
      return;
    }

    if (this.currentTargetUrl && this.autoOpenTargetTab) {
      const page = await this.createTargetPage(this.currentTargetUrl);
      await this.navigateTargetPage(page, this.currentTargetUrl);
    }
  }

  private attachPage(page: Page, activate = false): void {
    if (this.observedPages.has(page)) {
      this.updatePageLocationState(page);
      if (activate) {
        this.activateTargetPage(page, this.targetPageCreatedByAdapter);
      }
      return;
    }
    this.observedPages.add(page);
    const pageState = this.updatePageLocationState(page);
    const pageId = pageState.page_id;

    const mainFrame = page.mainFrame();
    let seenInitialNavigation = Boolean(
      page.url() && page.url() !== "about:blank",
    );
    page.on("framenavigated", (frame) => {
      if (frame !== mainFrame) {
        return;
      }
      const state = this.updatePageLocationState(page);
      if (!page.isClosed() && this.managedPage === page) {
        this.currentTargetUrl = page.url();
        this.targetPageId = state.page_id;
        this.applyPageStateToActiveTarget(state);
      }
      if (seenInitialNavigation) {
        this.reloadCount += 1;
      } else {
        seenInitialNavigation = true;
      }
    });
    page.on("close", () => {
      this.pageStates.delete(pageId);
      if (this.managedPage === page) {
        this.managedPage = undefined;
        this.targetPageId = null;
        this.targetPageCreatedByAdapter = false;
        this.clearActiveCapabilityState();
      }
    });
    page.on("crash", () => {
      this.pageStates.delete(pageId);
      if (this.managedPage === page) {
        this.managedPage = undefined;
        this.targetPageId = null;
        this.targetPageCreatedByAdapter = false;
        this.clearActiveCapabilityState();
      }
    });
    if (activate) {
      this.activateTargetPage(page, this.targetPageCreatedByAdapter);
    }
  }

  private async attachToExistingBrowser(): Promise<void> {
    const endpoint = this.debugEndpoint?.trim();
    if (!endpoint) {
      throw new Error(
        "cdp-unavailable: browser debug endpoint is not configured",
      );
    }

    try {
      this.browser = await chromium.connectOverCDP(endpoint);
    } catch (error) {
      throw new Error(
        `cdp-connect-failed: ${error instanceof Error ? error.message : String(error)}`,
      );
    }

    const context = this.browser.contexts()[0];
    if (!context) {
      await this.browser.close().catch(() => undefined);
      this.browser = undefined;
      throw new Error(
        "browser-not-debuggable: attached browser did not expose a debuggable context",
      );
    }

    this.browserOwner = "external";
    this.cdpAttached = true;
    this.context = context;
    this.observeContext(context);
    this.browser.on("disconnected", () => {
      this.browser = undefined;
      this.context = undefined;
      this.managedPage = undefined;
      this.targetPageId = null;
      this.targetPageCreatedByAdapter = false;
      this.cdpAttached = false;
      this.pageStates.clear();
      this.clearActiveCapabilityState();
    });
    for (const existingContext of this.browser.contexts()) {
      this.observeContext(existingContext);
      for (const page of existingContext.pages()) {
        this.attachPage(page);
      }
    }
  }

  private async launchManagedContext(
    headless: boolean,
    browserPath?: string,
  ): Promise<void> {
    const launch = resolveBrowserLaunchConfig(process.platform, browserPath);
    const launchArgs =
      !headless && this.extensionDir
        ? [
            `--disable-extensions-except=${this.extensionDir}`,
            `--load-extension=${this.extensionDir}`,
          ]
        : [];
    try {
      this.context = await chromium.launchPersistentContext(this.profileDir, {
        acceptDownloads: true,
        channel: launch.channel,
        executablePath: launch.executablePath,
        downloadsPath: this.downloadsDir,
        headless,
        args: launchArgs,
        ignoreDefaultArgs:
          launchArgs.length > 0 ? ["--disable-extensions"] : undefined,
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (
        message.includes("user data directory is already in use") ||
        message.includes("SingletonLock")
      ) {
        throw new Error(`profile locked: ${message}`);
      }
      throw new Error(`browser launch failed: ${message}`);
    }
    this.browser = undefined;
    this.browserOwner = "managed";
    this.cdpAttached = false;
    this.observeContext(this.context);
    for (const page of this.context.pages()) {
      this.attachPage(page, !this.managedPage);
    }
  }

  private observeContext(context: BrowserContext): void {
    if (this.observedContexts.has(context)) {
      return;
    }
    this.observedContexts.add(context);

    context.on("close", () => {
      if (this.context === context) {
        this.context = undefined;
        this.managedPage = undefined;
        this.targetPageId = null;
        this.targetPageCreatedByAdapter = false;
        this.clearActiveCapabilityState();
      }
    });
    context.on("page", (page) => {
      this.attachPage(page);
    });
  }

  private async createTargetPage(targetUrl: string): Promise<Page> {
    if (this.cdpAttached) {
      const existing = await this.findAttachedTargetPage(targetUrl);
      if (existing) {
        this.context = existing.context();
        this.observeContext(this.context);
        this.attachPage(existing, true);
        this.targetPageCreatedByAdapter = false;
        await existing.bringToFront().catch(() => undefined);
        return existing;
      }
      if (!this.autoOpenTargetTab) {
        throw new Error(
          "target-tab-missing: no matching target tab was found in the attached browser",
        );
      }
      await this.cleanupStaleAttachedTargetPages(targetUrl);
    }

    if (!this.context) {
      throw new Error(
        this.cdpAttached
          ? "target-tab-missing: attached browser context is unavailable"
          : "browser launch failed: browser context is unavailable",
      );
    }

    const page = await this.context.newPage();
    this.attachPage(page, true);
    this.targetPageCreatedByAdapter = true;
    return page;
  }

  private async navigateTargetPage(
    page: Page,
    targetUrl: string,
  ): Promise<void> {
    if (this.cdpAttached && matchesTargetLocation(page.url(), targetUrl)) {
      await page.waitForLoadState("domcontentloaded", {
        timeout: NAVIGATION_TIMEOUT_MS,
      });
      return;
    }

    if (page.url() !== targetUrl) {
      await page.goto(targetUrl, {
        waitUntil: "domcontentloaded",
        timeout: NAVIGATION_TIMEOUT_MS,
      });
      return;
    }

    await page.waitForLoadState("domcontentloaded", {
      timeout: NAVIGATION_TIMEOUT_MS,
    });
  }

  private async findAttachedTargetPage(
    targetUrl: string,
  ): Promise<Page | undefined> {
    if (!this.browser) {
      return undefined;
    }

    const workspaceCandidates = uniquePages(
      this.browser
      .contexts()
      .flatMap((context) => context.pages())
      .filter(
        (page) =>
          !page.isClosed() && this.pageMatchesTargetWorkspace(page, targetUrl),
      ),
    );
    const orderedCandidates = uniquePages([
      ...(this.managedPage && !this.managedPage.isClosed()
        ? [this.managedPage]
        : []),
      ...workspaceCandidates,
    ]);

    let workspaceFallback: Page | undefined;
    for (const page of orderedCandidates) {
      if (!(await this.isUsableAttachedTargetPage(page))) {
        continue;
      }
      if (this.pageMatchesTargetLocation(page, targetUrl)) {
        return page;
      }
      workspaceFallback ??= page;
    }

    return workspaceFallback;
  }

  private pageMatchesTargetLocation(page: Page, targetUrl: string): boolean {
    const currentUrl = safePageUrl(page);
    return Boolean(currentUrl && matchesTargetLocation(currentUrl, targetUrl));
  }

  private pageMatchesTargetWorkspace(page: Page, targetUrl: string): boolean {
    const currentUrl = safePageUrl(page);
    return Boolean(currentUrl && matchesTargetWorkspace(currentUrl, targetUrl));
  }

  private async isUsableAttachedTargetPage(page: Page): Promise<boolean> {
    const currentUrl = safePageUrl(page);
    if (!currentUrl || !inferAuthenticated(currentUrl)) {
      return false;
    }
    const state = await this.refreshCapabilities(page);
    return Boolean(
      state?.extension_connected &&
        state.capability_report &&
        hasUsableCapabilityReport(state.capability_report),
    );
  }

  private async cleanupStaleAttachedTargetPages(
    targetUrl: string,
    keepPage?: Page,
  ): Promise<void> {
    if (!this.browser) {
      return;
    }

    const candidates = uniquePages(
      this.browser
        .contexts()
        .flatMap((context) => context.pages())
        .filter(
          (page) =>
            !page.isClosed() &&
            page !== keepPage &&
            this.pageMatchesTargetWorkspace(page, targetUrl),
        ),
    );

    for (const page of candidates) {
      if (await this.isUsableAttachedTargetPage(page)) {
        continue;
      }
      await this.discardAttachedTargetPage(page);
    }
  }

  private async discardAttachedTargetPage(page: Page): Promise<void> {
    if (!this.cdpAttached || page.isClosed()) {
      return;
    }
    if (this.managedPage === page) {
      this.managedPage = undefined;
      this.targetPageId = null;
      this.targetPageCreatedByAdapter = false;
      this.clearActiveCapabilityState();
    }
    await this.closeAttachedTargetPage(page).catch(() => undefined);
  }

  private async closeAttachedTargetPage(page: Page): Promise<void> {
    try {
      const session = await page.context().newCDPSession(page);
      const info = await session.send("Target.getTargetInfo");
      const targetId = info?.targetInfo?.targetId;
      if (targetId) {
        await session.send("Target.closeTarget", { targetId });
        await session.detach().catch(() => undefined);
        return;
      }
    } catch {}

    await page.close().catch(() => undefined);
  }

  private async refreshCapabilities(
    page: Page,
  ): Promise<TargetPageState | undefined> {
    if (page.isClosed()) {
      return undefined;
    }

    const pageState = this.updatePageLocationState(page);
    const preferredStrategies = pageState.ui_signature
      ? (this.strategyTelemetry.get(pageState.ui_signature) ?? {})
      : (this.capabilityCache.workspaces[workspaceKeyFromUrl(page.url())]
          ?.preferred_strategies ?? {});
    const selectorPack = this.ensureSelectorPackLoaded();
    const probeDeadline = this.cdpAttached
      ? Math.max(pageState.probe_ready_until_ms, Date.now())
      : Date.now();

    for (let attempt = 0; attempt < 10; attempt += 1) {
      try {
        await page.waitForLoadState("domcontentloaded", {
          timeout: PROBE_TIMEOUT_MS,
        });
        const message = await page.evaluate(
          ({
            source,
            options,
          }: {
            source: string;
            options: ProbeDocumentOptions;
          }): CapabilityBridgeMessage => {
            const probeDocument = new Function(
              `${source}\nreturn probeDocument;`,
            )() as (options?: ProbeDocumentOptions) => CapabilityBridgeMessage;
            return probeDocument(options);
          },
          {
            source: loadProbeSource(),
            options: {
              selectorPackId: selectorPack?.pack_id ?? "chatgpt-default",
              selectorPackVersion: selectorPack
                ? `v${selectorPack.format_version}`
                : "v1",
              preferredStrategies,
              selectorPack: selectorPack ?? undefined,
            } satisfies ProbeDocumentOptions,
          },
        );
        if (!hasUsableCapabilityReport(message.report)) {
          if (this.cdpAttached && Date.now() < probeDeadline) {
            await sleep(150);
            continue;
          }
          if (this.cdpAttached) {
            return this.recordCapabilityFailure(page);
          }
        }
        return this.recordCapabilitySuccess(page, message);
      } catch (error) {
        if (attempt < 2 && isTransientPageError(error)) {
          await sleep(150 * (attempt + 1));
          continue;
        }
        if (this.cdpAttached && Date.now() < probeDeadline) {
          await sleep(150);
          continue;
        }
        return this.recordCapabilityFailure(page);
      }
    }

    return this.recordCapabilityFailure(page);
  }

  private hydrateCapabilityCache(): void {
    if (this.capabilityCacheLoaded) {
      return;
    }
    this.capabilityCacheLoaded = true;

    try {
      const parsed = JSON.parse(
        readFileSync(this.capabilityCachePath, "utf8"),
      ) as CapabilityCacheFile & {
        version?: number;
        telemetry?: StrategyTelemetryStat[];
      };
      if (!parsed?.workspaces) {
        this.capabilityCache = {
          version: 2,
          workspaces: {},
          telemetry: [],
        };
        return;
      }
      this.capabilityCache = {
        version: 2,
        workspaces: parsed.workspaces,
        telemetry: parsed.telemetry ?? [],
      };
      for (const entry of Object.values(this.capabilityCache.workspaces)) {
        if (entry.ui_signature) {
          this.strategyTelemetry.set(
            entry.ui_signature,
            entry.preferred_strategies ?? {},
          );
        }
      }
      for (const stat of this.capabilityCache.telemetry) {
        this.strategyTelemetryStats.set(
          telemetryKey(stat.capability, stat.strategy_id),
          stat,
        );
      }
    } catch {
      this.capabilityCache = {
        version: 2,
        workspaces: {},
        telemetry: [],
      };
    }
  }

  private persistCapabilitySnapshot(
    currentUrl: string,
    message: CapabilityBridgeMessage,
  ): void {
    const workspaceKey = workspaceKeyFromUrl(currentUrl);
    this.capabilityCache.workspaces[workspaceKey] = {
      workspace_key: workspaceKey,
      saved_at: new Date().toISOString(),
      ui_signature: message.runtime.ui_signature,
      preferred_strategies: message.runtime.strategy_matches,
      report: message.report,
    };
    this.capabilityCache.telemetry = Array.from(
      this.strategyTelemetryStats.values(),
    );

    try {
      writeFileSync(
        this.capabilityCachePath,
        JSON.stringify(this.capabilityCache, null, 2),
        "utf8",
      );
    } catch {
      // Cache persistence is opportunistic and should not fail browser control.
    }
  }

  private ensureSelectorPackLoaded(): SelectorPackSnapshot | null {
    if (this.selectorPack) {
      return this.selectorPack;
    }

    try {
      const parsed = JSON.parse(
        readFileSync(this.selectorPackPath, "utf8"),
      ) as SelectorPackSnapshot;
      if (!parsed?.pack_id || parsed.format_version !== 1) {
        return null;
      }
      this.selectorPack = parsed;
      return this.selectorPack;
    } catch {
      return null;
    }
  }

  private recordStrategyStats(
    strategyMatches: Partial<Record<CapabilityKey, string>>,
  ): void {
    const capabilities = [
      "composer_ready",
      "submit_available",
      "model_picker_visible",
      "thinking_effort_visible",
      "search_mode_visible",
      "research_mode_visible",
      "upload_control_visible",
      "downloadable_artifacts_observable",
    ] satisfies CapabilityKey[];

    for (const capability of capabilities) {
      const matched = strategyMatches[capability];
      if (matched) {
        const key = telemetryKey(capability, matched);
        const current = this.strategyTelemetryStats.get(key) ?? {
          capability,
          strategy_id: matched,
          success_count: 0,
          failure_count: 0,
          last_used_at: null,
        };
        current.success_count += 1;
        current.last_used_at = new Date().toISOString();
        this.strategyTelemetryStats.set(key, current);
        continue;
      }

      const preferred = this.lastUiSignature
        ? this.strategyTelemetry.get(this.lastUiSignature)?.[capability]
        : undefined;
      const strategyId =
        preferred ??
        this.ensureSelectorPackLoaded()?.capabilities?.[capability]?.[0]
          ?.strategy_id;
      if (!strategyId) {
        continue;
      }
      const key = telemetryKey(capability, strategyId);
      const current = this.strategyTelemetryStats.get(key) ?? {
        capability,
        strategy_id: strategyId,
        success_count: 0,
        failure_count: 0,
        last_used_at: null,
      };
      current.failure_count += 1;
      this.strategyTelemetryStats.set(key, current);
    }
  }

  private buildStrategyAttempts(): StrategyAttemptSnapshot[] {
    const selectorPack = this.ensureSelectorPackLoaded();
    const matrix = this.lastCapabilityReport?.matrix;
    const matched = this.lastUiSignature
      ? (this.strategyTelemetry.get(this.lastUiSignature) ?? {})
      : {};
    const attempts: StrategyAttemptSnapshot[] = [];
    const capabilities = Object.keys(
      selectorPack?.capabilities ?? {},
    ) as CapabilityKey[];

    for (const capability of capabilities) {
      const strategies = selectorPack?.capabilities?.[capability] ?? [];
      const matchedStrategy = matched[capability];
      if (matchedStrategy) {
        const strategy = strategies.find(
          (entry) => entry.strategy_id === matchedStrategy,
        );
        attempts.push({
          capability,
          strategy_id: matchedStrategy,
          score: strategy?.score ?? 1,
          outcome: "matched",
          detail:
            this.lastDomNeighborhoods.find(
              (entry) =>
                entry.capability === capability &&
                entry.strategy_id === matchedStrategy,
            )?.anchor_path ?? null,
        });
        continue;
      }

      if (matrix && !matrix[capability]) {
        const first = strategies[0];
        if (first) {
          attempts.push({
            capability,
            strategy_id: first.strategy_id,
            score: first.score,
            outcome: "missing",
            detail: "probe did not find a matching visible control",
          });
        }
      }
    }

    return attempts;
  }

  private getStrategyTelemetrySnapshots(): StrategyTelemetrySnapshot[] {
    return Array.from(this.strategyTelemetryStats.values())
      .sort((left, right) => {
        if (left.capability === right.capability) {
          return left.strategy_id.localeCompare(right.strategy_id);
        }
        return left.capability.localeCompare(right.capability);
      })
      .map((entry) => ({
        capability: entry.capability,
        strategy_id: entry.strategy_id,
        success_count: entry.success_count,
        failure_count: entry.failure_count,
        success_rate:
          entry.success_count + entry.failure_count === 0
            ? 0
            : entry.success_count / (entry.success_count + entry.failure_count),
        last_used_at: entry.last_used_at,
      }));
  }

  private async captureBundleScreenshot(
    page: Page,
    bundleDir: string,
  ): Promise<string> {
    const screenshotPath = resolve(bundleDir, "failure.png");
    await mkdir(dirname(screenshotPath), { recursive: true });
    await page.screenshot({ fullPage: true, path: screenshotPath });
    this.lastScreenshotPath = screenshotPath;
    return screenshotPath;
  }
}

