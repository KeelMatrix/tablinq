import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { mkdir, readFile, stat } from "node:fs/promises";
import { basename, dirname, extname, resolve } from "node:path";
import {
  type Browser,
  chromium,
  type BrowserContext,
  type Download,
  type Locator,
  type Page,
} from "playwright-core";

type CapabilityKey =
  | "composer_ready"
  | "submit_available"
  | "model_picker_visible"
  | "thinking_effort_visible"
  | "search_mode_visible"
  | "research_mode_visible"
  | "upload_control_visible"
  | "downloadable_artifacts_observable";

interface CapabilityMatrixSnapshot {
  composer_ready: boolean;
  submit_available: boolean;
  model_picker_visible: boolean;
  thinking_effort_visible: boolean;
  search_mode_visible: boolean;
  research_mode_visible: boolean;
  upload_control_visible: boolean;
  downloadable_artifacts_observable: boolean;
}

interface RuntimeAliasesSnapshot {
  modes: Record<string, string[]>;
  model_classes: Record<string, string[]>;
  efforts: Record<string, string[]>;
}

interface CapabilityReportPayload {
  reported_at: string;
  selector_pack_id: string;
  selector_pack_version: string;
  matrix: CapabilityMatrixSnapshot;
  aliases: RuntimeAliasesSnapshot;
}

interface DomNeighborhoodSnapshot {
  capability: CapabilityKey;
  strategy_id: string;
  anchor_tag: string;
  anchor_role: string | null;
  anchor_name: string;
  anchor_path: string;
  parent_name: string | null;
  sibling_names: string[];
  nearby_text: string[];
}

interface CapabilityBridgeMessage {
  type: "capability_report";
  report: CapabilityReportPayload;
  runtime: {
    ui_signature: string;
    strategy_matches: Partial<Record<CapabilityKey, string>>;
    dom_neighborhoods: DomNeighborhoodSnapshot[];
  };
}

interface SelectorStrategySnapshot {
  strategy_id: string;
  kind: "aria_role" | "css" | "text" | "dom_neighborhood" | "hotkey";
  selector: string;
  score: number;
}

interface SelectorPackSnapshot {
  format_version: number;
  pack_id: string;
  description: string;
  updated_at: string;
  capabilities: Partial<Record<CapabilityKey, SelectorStrategySnapshot[]>>;
}

interface ProbeDocumentOptions {
  selectorPackId?: string;
  selectorPackVersion?: string;
  reportedAt?: string;
  preferredStrategies?: Partial<Record<CapabilityKey, string>>;
  selectorPack?: SelectorPackSnapshot;
}

let probeSource: string | undefined;
let pageAdapterSource: string | undefined;

const NAVIGATION_TIMEOUT_MS = 20_000;
const ASK_TIMEOUT_MS = 25_000;
const RESEARCH_TIMEOUT_MS = 60_000;
const PROBE_TIMEOUT_MS = 5_000;
const ATTACH_PROBE_GRACE_MS = 1_500;
const INTERACTION_TIMEOUT_MS = 7_500;
const UPLOAD_ACK_TIMEOUT_MS = 10_000;
const DOWNLOAD_TIMEOUT_MS = 10_000;
const SNAPSHOT_POLL_INTERVAL_MS = 75;
const STABLE_DONE_POLLS = 3;
const STALL_DETECTION_MIN_MS = 500;
const STALL_DETECTION_MAX_MS = 2_000;
const MAX_RECOVERY_ATTEMPTS = 3;

function loadProbeSource(): string {
  if (!probeSource) {
    probeSource = readFileSync(
      resolveTablinqAssetPath(
        ["extension", "src", "probe.runtime.js"],
        ["packages", "extension", "src", "probe.runtime.js"],
      ),
      "utf8",
    ).replace(/^export\s+function\s+probeDocument/m, "function probeDocument");
  }

  return probeSource;
}

function loadPageAdapterSource(): string {
  if (!pageAdapterSource) {
    pageAdapterSource = readFileSync(
      resolveTablinqAssetPath(
        ["browser-adapter", "src", "pageAdapter.runtime.js"],
        ["packages", "browser-adapter", "src", "pageAdapter.runtime.js"],
      ),
      "utf8",
    ).replace(
      /^export\s+function\s+createTablinqPageAdapterRuntime/m,
      "function createTablinqPageAdapterRuntime",
    );
  }

  return pageAdapterSource;
}

export interface BrowserOpenCommand {
  targetUrl: string;
  browserPath?: string | null;
  headless?: boolean;
}

export interface AskExecutionCommand {
  jobId: string;
  prompt: string;
  conversationId?: string | null;
  timeoutMs?: number;
  mode?: "normal" | "search" | "research";
  modelClass?: "instant" | "thinking" | null;
  effort?: "none" | "standard" | "extended" | null;
  files?: string[];
}

export interface ResearchExecutionCommand {
  jobId: string;
  query: string;
  conversationId?: string | null;
  files?: string[];
  outputDir?: string | null;
  timeoutMs?: number;
}

export interface ArtifactMetadataSnapshot {
  artifact_id: string;
  conversation_id: string | null;
  message_id: string | null;
  file_name: string;
  content_type: string;
  size_bytes: number;
  sha256: string;
  storage_path: string;
}

export interface AskResultSnapshot {
  job_id: string;
  conversation_id: string;
  message_id: string;
  output_text: string;
  artifacts: ArtifactMetadataSnapshot[];
  completed_at: string;
  current_url: string;
}

export interface ResearchProgressSnapshot {
  status: "starting" | "waiting_for_sources" | "running";
  message?: string;
  conversation_id: string | null;
  message_id: string | null;
  current_url: string;
  chunk_kind?: "delta" | "status";
  chunk_text?: string;
}

export interface ResearchResultSnapshot {
  job_id: string;
  conversation_id: string;
  message_id: string;
  report_text: string;
  artifacts: ArtifactMetadataSnapshot[];
  completed_at: string;
  current_url: string;
  output_path?: string | null;
}

export interface StreamChunkSnapshot {
  job_id: string;
  conversation_id: string | null;
  message_id: string | null;
  seq: number;
  kind: "delta" | "done";
  text: string;
}

export interface RequestStateSnapshot {
  state: "interrupted" | "recovered";
  reason: string;
  at: string;
  conversation_id: string | null;
  message_id: string | null;
  current_url: string | null;
  streaming_safe: boolean;
}

export interface BrowserAdapterOptions {
  profileDir: string;
  downloadsDir: string;
  defaultTargetUrl: string;
  defaultBrowserPath?: string | null;
  extensionDir?: string | null;
  connectionMode?: "attach_first" | "managed_fallback" | "managed_only" | null;
  debugEndpoint?: string | null;
  extensionRequired?: boolean;
  autoOpenTargetTab?: boolean;
}

export interface BrowserStatusSnapshot {
  running: boolean;
  browser_kind: string;
  profile_dir: string;
  downloads_dir: string;
  target_url: string;
  current_url: string | null;
  page_title: string | null;
  page_ready: boolean;
  authenticated: boolean;
  reload_count: number;
  screenshot_path: string | null;
  browser_path: string | null;
  extension_connected: boolean;
  capability_report: CapabilityReportPayload | null;
  attach_mode: "attach_first" | "managed_fallback" | "managed_only";
  browser_owner: "external" | "managed";
  cdp_attached: boolean;
  target_tab_found: boolean;
  target_tab_id: string | null;
  target_workspace_key: string | null;
  debug_endpoint: string | null;
  auth_source: "page" | "extension" | "unknown";
  extension_required: boolean;
  last_capability_report_at: string | null;
}

export interface BrowserLaunchConfig {
  browserKind: string;
  channel?: "msedge" | "chrome";
  executablePath?: string;
}

export interface StrategyAttemptSnapshot {
  capability: string;
  strategy_id: string;
  score: number;
  outcome: string;
  detail: string | null;
}

export interface StrategyTelemetrySnapshot {
  capability: string;
  strategy_id: string;
  success_count: number;
  failure_count: number;
  success_rate: number;
  last_used_at: string | null;
}

export interface SelectorPackSummarySnapshot {
  pack_id: string;
  version: string;
  updated_at: string | null;
  path: string | null;
}

export interface BrowserDiagnosticsSnapshot {
  current_url: string | null;
  capability_matrix: CapabilityMatrixSnapshot;
  strategy_attempts: StrategyAttemptSnapshot[];
  dom_snapshot: string;
  aria_snapshot: string;
  screenshot_path: string | null;
  selector_pack: SelectorPackSummarySnapshot | null;
  strategy_telemetry: StrategyTelemetrySnapshot[];
  browser_detected: boolean;
  detected_browser_path: string | null;
  profile_accessible: boolean;
  profile_access_error: string | null;
}

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
    return this.runAsk(command, onChunk, onState, {
      submitPrompt: true,
      emittedText: "",
      nextSeq: 0,
      recoveryReason: null,
    });
  }

  async research(
    command: ResearchExecutionCommand,
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<ResearchResultSnapshot> {
    return this.runResearch(command, onUpdate, onState, {
      submitPrompt: true,
      emittedText: "",
      nextSeq: 0,
      recoveryReason: null,
    });
  }

  async recoverAsk(
    command: AskExecutionCommand,
    state: { emittedText: string; nextSeq: number },
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<AskResultSnapshot> {
    return this.runAsk(command, onChunk, onState, {
      submitPrompt: false,
      emittedText: state.emittedText,
      nextSeq: state.nextSeq,
      recoveryReason: "re-attached to the browser session",
    });
  }

  async recoverResearch(
    command: ResearchExecutionCommand,
    state: { emittedText: string; nextSeq: number },
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<ResearchResultSnapshot> {
    return this.runResearch(command, onUpdate, onState, {
      submitPrompt: false,
      emittedText: state.emittedText,
      nextSeq: state.nextSeq,
      recoveryReason: "re-attached to the browser session",
    });
  }

  private async runAsk(
    command: AskExecutionCommand,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void,
    options: {
      submitPrompt: boolean;
      emittedText: string;
      nextSeq: number;
      recoveryReason: string | null;
    },
  ): Promise<AskResultSnapshot> {
    const timeoutAt = Date.now() + (command.timeoutMs ?? ASK_TIMEOUT_MS);
    const stallTimeoutMs = computeStallTimeoutMs(
      command.timeoutMs,
      ASK_TIMEOUT_MS,
    );
    let page = await this.openExecutionPage(
      this.resolveConversationTarget(command.conversationId),
    );

    if (options.submitPrompt) {
      if (!command.conversationId) {
        await resetConversationIdentity(page);
      }
      await this.refreshCapabilities(page);
      await applyAskConfiguration(page, command, this.lastCapabilityReport);
      await uploadAttachments(
        page,
        command.files ?? [],
        this.lastCapabilityReport,
      );
      await fillComposer(page, command.prompt);
      await clickSubmit(page);
    }

    const state = {
      emittedText: options.emittedText,
      nextSeq: options.nextSeq,
      doneStreak: 0,
      knownConversationId: command.conversationId ?? null,
      knownMessageId: null as string | null,
      lastProgressAt: Date.now(),
      recoveryAttempts: 0,
      lastStatusAt: 0,
    };

    if (!options.submitPrompt) {
      const recovered = await this.reconcileAskAfterInterruption(
        page,
        command,
        state,
        onChunk,
        onState,
        options.recoveryReason ?? "recovered the interrupted ask",
        timeoutAt,
      );
      page = recovered.page;
      if (recovered.result) {
        return recovered.result;
      }
    }

    while (Date.now() < timeoutAt) {
      let snapshot: AskSnapshot;
      try {
        snapshot = await readAskSnapshot(page);
      } catch (error) {
        if (
          isTransientPageError(error) &&
          state.recoveryAttempts < MAX_RECOVERY_ATTEMPTS
        ) {
          const recovered = await this.reconcileAskAfterInterruption(
            page,
            command,
            state,
            onChunk,
            onState,
            error instanceof Error ? error.message : String(error),
            timeoutAt,
          );
          page = recovered.page;
          if (recovered.result) {
            return recovered.result;
          }
          await sleep(SNAPSHOT_POLL_INTERVAL_MS);
          continue;
        }
        throw error;
      }

      this.rememberAskSnapshot(state, snapshot);
      const progressed = await this.applyAskSnapshot(
        command,
        state,
        snapshot,
        onChunk,
      );
      if (progressed) {
        state.lastProgressAt = Date.now();
      }

      if (snapshot.state === "done") {
        state.doneStreak += 1;
      } else {
        state.doneStreak = 0;
      }

      if (snapshot.state === "done" && state.doneStreak >= STABLE_DONE_POLLS) {
        return this.finalizeAsk(command, page, state, snapshot, onChunk);
      }

      const canRecoverFromAskStall =
        state.emittedText.length > 0 ||
        state.knownConversationId !== null ||
        state.knownMessageId !== null;
      if (
        Date.now() - state.lastProgressAt >= stallTimeoutMs &&
        state.recoveryAttempts < MAX_RECOVERY_ATTEMPTS &&
        canRecoverFromAskStall
      ) {
        const recovered = await this.reconcileAskAfterInterruption(
          page,
          command,
          state,
          onChunk,
          onState,
          "stream stalled while waiting for provider output",
          timeoutAt,
        );
        page = recovered.page;
        if (recovered.result) {
          return recovered.result;
        }
      }

      await sleep(SNAPSHOT_POLL_INTERVAL_MS);
    }

    throw new Error("ask timed out waiting for streamed response");
  }

  private async runResearch(
    command: ResearchExecutionCommand,
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void,
    options: {
      submitPrompt: boolean;
      emittedText: string;
      nextSeq: number;
      recoveryReason: string | null;
    },
  ): Promise<ResearchResultSnapshot> {
    const timeoutAt = Date.now() + (command.timeoutMs ?? RESEARCH_TIMEOUT_MS);
    const stallTimeoutMs = computeStallTimeoutMs(
      command.timeoutMs,
      RESEARCH_TIMEOUT_MS,
    );
    let page = await this.openExecutionPage(
      this.resolveConversationTarget(command.conversationId),
    );

    if (options.submitPrompt) {
      if (!command.conversationId) {
        await resetConversationIdentity(page);
      }
      await this.refreshCapabilities(page);
      await applyAskConfiguration(
        page,
        {
          jobId: command.jobId,
          prompt: command.query,
          mode: "research",
        },
        this.lastCapabilityReport,
      );
      await uploadAttachments(
        page,
        command.files ?? [],
        this.lastCapabilityReport,
      );
      await fillComposer(page, command.query);
      await clickSubmit(page);
    }

    const state = {
      emittedText: options.emittedText,
      nextSeq: options.nextSeq,
      lastStatus: "starting",
      lastMessage: "starting research",
      doneStreak: 0,
      knownConversationId: command.conversationId ?? null,
      knownMessageId: null as string | null,
      lastProgressAt: Date.now(),
      recoveryAttempts: 0,
      announcedStart: false,
      lastStatusAt: 0,
    };

    if (options.submitPrompt) {
      await onUpdate({
        status: "starting",
        message: "starting research",
        conversation_id: null,
        message_id: null,
        current_url: page.url(),
      });
      state.announcedStart = true;
    }

    if (!options.submitPrompt) {
      const recovered = await this.reconcileResearchAfterInterruption(
        page,
        command,
        state,
        onUpdate,
        onState,
        options.recoveryReason ?? "recovered the interrupted research job",
        timeoutAt,
      );
      page = recovered.page;
      if (recovered.result) {
        return recovered.result;
      }
    }

    while (Date.now() < timeoutAt) {
      let snapshot: ResearchSnapshot;
      try {
        snapshot = await readResearchSnapshot(page);
      } catch (error) {
        if (
          isTransientPageError(error) &&
          state.recoveryAttempts < MAX_RECOVERY_ATTEMPTS
        ) {
          const recovered = await this.reconcileResearchAfterInterruption(
            page,
            command,
            state,
            onUpdate,
            onState,
            error instanceof Error ? error.message : String(error),
            timeoutAt,
          );
          page = recovered.page;
          if (recovered.result) {
            return recovered.result;
          }
          await sleep(SNAPSHOT_POLL_INTERVAL_MS);
          continue;
        }
        throw error;
      }

      this.rememberResearchSnapshot(state, snapshot);
      const progressed = await this.applyResearchSnapshot(
        state,
        snapshot,
        onUpdate,
      );
      if (progressed) {
        state.lastProgressAt = Date.now();
      }

      if (snapshot.status === "failed") {
        throw new Error(
          `research job failed: ${snapshot.statusMessage ?? "research failed"}`,
        );
      }

      if (snapshot.status === "done") {
        state.doneStreak += 1;
      } else {
        state.doneStreak = 0;
      }

      if (snapshot.status === "done" && state.doneStreak >= STABLE_DONE_POLLS) {
        return this.finalizeResearch(command, page, state, snapshot);
      }

      if (
        Date.now() - state.lastProgressAt >= stallTimeoutMs &&
        state.recoveryAttempts < MAX_RECOVERY_ATTEMPTS
      ) {
        const recovered = await this.reconcileResearchAfterInterruption(
          page,
          command,
          state,
          onUpdate,
          onState,
          "stream stalled while waiting for provider output",
          timeoutAt,
        );
        page = recovered.page;
        if (recovered.result) {
          return recovered.result;
        }
      }

      await sleep(SNAPSHOT_POLL_INTERVAL_MS);
    }

    throw new Error("research job failed: timed out waiting for completion");
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

  private rememberAskSnapshot(
    state: {
      knownConversationId: string | null;
      knownMessageId: string | null;
    },
    snapshot: AskSnapshot,
  ): void {
    if (snapshot.currentUrl) {
      this.currentTargetUrl = snapshot.currentUrl;
    }
    if (snapshot.conversationId) {
      state.knownConversationId = snapshot.conversationId;
    }
    if (snapshot.messageId) {
      state.knownMessageId = snapshot.messageId;
    }
  }

  private rememberResearchSnapshot(
    state: {
      knownConversationId: string | null;
      knownMessageId: string | null;
    },
    snapshot: ResearchSnapshot,
  ): void {
    if (snapshot.currentUrl) {
      this.currentTargetUrl = snapshot.currentUrl;
    }
    if (snapshot.conversationId) {
      state.knownConversationId = snapshot.conversationId;
    }
    if (snapshot.messageId) {
      state.knownMessageId = snapshot.messageId;
    }
  }

  private async applyAskSnapshot(
    command: AskExecutionCommand,
    state: { emittedText: string; nextSeq: number },
    snapshot: AskSnapshot,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
  ): Promise<boolean> {
    if (
      snapshot.text.length > state.emittedText.length &&
      snapshot.text.startsWith(state.emittedText)
    ) {
      const delta = snapshot.text.slice(state.emittedText.length);
      state.emittedText = snapshot.text;
      await onChunk({
        job_id: command.jobId,
        conversation_id: snapshot.conversationId,
        message_id: snapshot.messageId,
        seq: state.nextSeq,
        kind: "delta",
        text: delta,
      });
      state.nextSeq += 1;
      return true;
    }
    return false;
  }

  private async applyResearchSnapshot(
    state: {
      emittedText: string;
      nextSeq: number;
      lastStatus: string;
      lastMessage: string;
      announcedStart: boolean;
    },
    snapshot: ResearchSnapshot,
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
  ): Promise<boolean> {
    let progressed = false;
    if (!state.announcedStart) {
      await onUpdate({
        status: "starting",
        message: "starting research",
        conversation_id: snapshot.conversationId,
        message_id: snapshot.messageId,
        current_url: snapshot.currentUrl,
      });
      state.announcedStart = true;
    }
    if (
      snapshot.status !== state.lastStatus ||
      snapshot.statusMessage !== state.lastMessage
    ) {
      state.lastStatus = snapshot.status;
      state.lastMessage = snapshot.statusMessage ?? "";
      if (
        snapshot.status === "waiting_for_sources" ||
        snapshot.status === "running"
      ) {
        await onUpdate({
          status: snapshot.status,
          message: snapshot.statusMessage ?? undefined,
          conversation_id: snapshot.conversationId,
          message_id: snapshot.messageId,
          current_url: snapshot.currentUrl,
        });
      }
    }
    if (
      snapshot.reportText.length > state.emittedText.length &&
      snapshot.reportText.startsWith(state.emittedText)
    ) {
      const delta = snapshot.reportText.slice(state.emittedText.length);
      state.emittedText = snapshot.reportText;
      await onUpdate({
        status: "running",
        message: snapshot.statusMessage ?? undefined,
        conversation_id: snapshot.conversationId,
        message_id: snapshot.messageId,
        current_url: snapshot.currentUrl,
        chunk_kind: "delta",
        chunk_text: delta,
      });
      state.nextSeq += 1;
      progressed = true;
    }
    return progressed;
  }

  private async reconcileAskAfterInterruption(
    page: Page,
    command: AskExecutionCommand,
    state: {
      emittedText: string;
      nextSeq: number;
      doneStreak: number;
      knownConversationId: string | null;
      knownMessageId: string | null;
      recoveryAttempts: number;
      lastStatusAt: number;
    },
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void,
    reason: string,
    timeoutAt: number,
  ): Promise<{ page: Page; result?: AskResultSnapshot }> {
    state.recoveryAttempts += 1;
    const interruptedAt = new Date().toISOString();
    if (Date.now() - state.lastStatusAt > 50) {
      await onState({
        state: "interrupted",
        reason,
        at: interruptedAt,
        conversation_id: state.knownConversationId,
        message_id: state.knownMessageId,
        current_url: safePageUrl(page),
        streaming_safe: false,
      });
      state.lastStatusAt = Date.now();
    }

    const recoveredPage = await this.openExecutionPage(
      this.resolveConversationTarget(
        state.knownConversationId ?? command.conversationId,
      ),
    );
    if (Date.now() >= timeoutAt) {
      throw new Error(
        "ask timed out while reconciling the interrupted request",
      );
    }

    const snapshot = await readAskSnapshot(recoveredPage);
    this.rememberAskSnapshot(state, snapshot);
    const relation = textRelation(state.emittedText, snapshot.text);
    const hasIdentity =
      Boolean(snapshot.conversationId || state.knownConversationId) ||
      snapshot.currentUrl.includes("conversation=");
    const canContinue =
      relation !== "mismatch" &&
      (snapshot.state === "running" || snapshot.state === "done") &&
      (hasIdentity || state.emittedText.length > 0 || snapshot.text.length > 0);
    if (!canContinue) {
      throw new Error(
        "interrupted ask could not be reconciled against the live conversation",
      );
    }

    await onState({
      state: "recovered",
      reason:
        snapshot.state === "done"
          ? "recovered the final assistant reply from the live conversation"
          : "recovered the in-flight assistant reply from the live conversation",
      at: new Date().toISOString(),
      conversation_id: snapshot.conversationId,
      message_id: snapshot.messageId,
      current_url: snapshot.currentUrl,
      streaming_safe: snapshot.state === "running",
    });

    const progressed = await this.applyAskSnapshot(
      command,
      state,
      snapshot,
      onChunk,
    );
    if (progressed) {
      state.doneStreak = 0;
    }
    if (snapshot.state === "done") {
      state.doneStreak = STABLE_DONE_POLLS;
      return {
        page: recoveredPage,
        result: await this.finalizeAsk(
          command,
          recoveredPage,
          state,
          snapshot,
          onChunk,
        ),
      };
    }
    return { page: recoveredPage };
  }

  private async reconcileResearchAfterInterruption(
    page: Page,
    command: ResearchExecutionCommand,
    state: {
      emittedText: string;
      nextSeq: number;
      lastStatus: string;
      lastMessage: string;
      doneStreak: number;
      knownConversationId: string | null;
      knownMessageId: string | null;
      recoveryAttempts: number;
      announcedStart: boolean;
      lastStatusAt: number;
    },
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void,
    reason: string,
    timeoutAt: number,
  ): Promise<{ page: Page; result?: ResearchResultSnapshot }> {
    state.recoveryAttempts += 1;
    const interruptedAt = new Date().toISOString();
    if (Date.now() - state.lastStatusAt > 50) {
      await onState({
        state: "interrupted",
        reason,
        at: interruptedAt,
        conversation_id: state.knownConversationId,
        message_id: state.knownMessageId,
        current_url: safePageUrl(page),
        streaming_safe: false,
      });
      state.lastStatusAt = Date.now();
    }

    const recoveredPage = await this.openExecutionPage(
      this.resolveConversationTarget(
        state.knownConversationId ?? command.conversationId,
      ),
    );
    if (Date.now() >= timeoutAt) {
      throw new Error(
        "research job failed: timed out while reconciling the interrupted request",
      );
    }

    const snapshot = await readResearchSnapshot(recoveredPage);
    this.rememberResearchSnapshot(state, snapshot);
    const relation = textRelation(state.emittedText, snapshot.reportText);
    const hasIdentity =
      Boolean(snapshot.conversationId || state.knownConversationId) ||
      snapshot.currentUrl.includes("conversation=");
    const canContinue =
      relation !== "mismatch" &&
      (snapshot.status === "waiting_for_sources" ||
        snapshot.status === "running" ||
        snapshot.status === "done") &&
      (hasIdentity ||
        state.emittedText.length > 0 ||
        snapshot.reportText.length > 0);
    if (!canContinue) {
      throw new Error(
        "interrupted research job could not be reconciled against the live conversation",
      );
    }

    await onState({
      state: "recovered",
      reason:
        snapshot.status === "done"
          ? "recovered the final research result from the live conversation"
          : "recovered the in-flight research result from the live conversation",
      at: new Date().toISOString(),
      conversation_id: snapshot.conversationId,
      message_id: snapshot.messageId,
      current_url: snapshot.currentUrl,
      streaming_safe:
        snapshot.status !== "done" && snapshot.status !== "failed",
    });

    const progressed = await this.applyResearchSnapshot(
      state,
      snapshot,
      onUpdate,
    );
    if (progressed) {
      state.doneStreak = 0;
    }
    if (snapshot.status === "done") {
      state.doneStreak = STABLE_DONE_POLLS;
      return {
        page: recoveredPage,
        result: await this.finalizeResearch(
          command,
          recoveredPage,
          state,
          snapshot,
        ),
      };
    }
    return { page: recoveredPage };
  }

  private async finalizeAsk(
    command: AskExecutionCommand,
    page: Page,
    state: { emittedText: string; nextSeq: number },
    snapshot: AskSnapshot,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
  ): Promise<AskResultSnapshot> {
    const artifacts = await collectArtifacts(
      page,
      this.downloadsDir,
      snapshot.conversationId,
      snapshot.messageId,
    );
    await this.ensureManagedPage(
      this.resolveConversationTarget(snapshot.conversationId),
    );
    await onChunk({
      job_id: command.jobId,
      conversation_id: snapshot.conversationId,
      message_id: snapshot.messageId,
      seq: state.nextSeq,
      kind: "done",
      text: "",
    });
    return {
      job_id: command.jobId,
      conversation_id:
        snapshot.conversationId ?? `${command.jobId}-conversation`,
      message_id: snapshot.messageId ?? `${command.jobId}-message`,
      output_text: snapshot.text,
      artifacts,
      completed_at: new Date().toISOString(),
      current_url: snapshot.currentUrl,
    };
  }

  private async finalizeResearch(
    command: ResearchExecutionCommand,
    page: Page,
    _state: { emittedText: string; nextSeq: number },
    snapshot: ResearchSnapshot,
  ): Promise<ResearchResultSnapshot> {
    const artifacts = await collectArtifacts(
      page,
      this.downloadsDir,
      snapshot.conversationId,
      snapshot.messageId,
    );
    await this.ensureManagedPage(
      this.resolveConversationTarget(snapshot.conversationId),
    );
    return {
      job_id: command.jobId,
      conversation_id:
        snapshot.conversationId ?? `${command.jobId}-conversation`,
      message_id: snapshot.messageId ?? `${command.jobId}-message`,
      report_text: snapshot.reportText,
      artifacts,
      completed_at: new Date().toISOString(),
      current_url: snapshot.currentUrl,
      output_path: null,
    };
  }

  private async ensureManagedPage(targetUrl: string): Promise<Page> {
    await this.recoverManagedPage();
    let page = this.managedPage;

    if (!page || page.isClosed()) {
      page = await this.createTargetPage(targetUrl);
    }

    try {
      await this.navigateTargetPage(page, targetUrl);
    } catch (error) {
      if (!isTransientPageError(error)) {
        throw error;
      }

      page = await this.createTargetPage(targetUrl);
      await this.navigateTargetPage(page, targetUrl);
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
    const target = new URL(this.currentTargetUrl);
    if (!conversationId) {
      target.searchParams.delete("conversation");
      return target.toString();
    }
    target.searchParams.set("conversation", conversationId);
    return target.toString();
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
    if (this.cdpAttached && matchesTargetWorkspace(page.url(), targetUrl)) {
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

    const candidates = this.browser
      .contexts()
      .flatMap((context) => context.pages())
      .filter(
        (page) =>
          !page.isClosed() && matchesTargetWorkspace(page.url(), targetUrl),
      );

    if (
      this.managedPage &&
      !this.managedPage.isClosed() &&
      candidates.includes(this.managedPage)
    ) {
      const activeState = await this.refreshCapabilities(this.managedPage);
      if (activeState?.extension_connected) {
        return this.managedPage;
      }
    }

    let fallback: Page | undefined;
    for (const page of candidates) {
      const state = await this.refreshCapabilities(page);
      if (state?.extension_connected) {
        return page;
      }
      fallback = page;
    }

    return fallback ?? candidates.at(-1);
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

export function resolveBrowserLaunchConfig(
  platform: NodeJS.Platform,
  browserPath?: string,
): BrowserLaunchConfig {
  if (browserPath) {
    return {
      browserKind: inferBrowserKind(browserPath),
      executablePath: browserPath,
    };
  }

  const detected = detectBrowserExecutable(platform);
  if (detected) {
    return {
      browserKind: inferBrowserKind(detected),
      executablePath: detected,
    };
  }

  throw new Error(
    "no supported browser executable found; set an explicit browser path",
  );
}

export function detectBrowserExecutable(
  platform: NodeJS.Platform,
): string | null {
  const envPath = process.env.TABLINQ_BROWSER_PATH?.trim();
  if (envPath) {
    return envPath;
  }

  if (platform === "win32") {
    return detectWindowsBrowserExecutable();
  }

  const candidates =
    platform === "darwin"
      ? [
          "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
          "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        ]
      : platform === "linux"
        ? ["google-chrome", "chromium", "chromium-browser", "microsoft-edge"]
        : [];

  for (const candidate of candidates) {
    try {
      if (platform === "linux") {
        const resolved = execFileSync("which", [candidate], {
          encoding: "utf8",
        }).trim();
        if (resolved) {
          return resolved;
        }
      } else {
        return candidate;
      }
    } catch {
      continue;
    }
  }

  return null;
}

function detectBrowserAvailability(
  platform: NodeJS.Platform,
  browserPath?: string,
): { detected: boolean; path: string | null } {
  if (browserPath) {
    return { detected: true, path: browserPath };
  }
  const detected = detectBrowserExecutable(platform);
  return {
    detected: Boolean(detected),
    path: detected,
  };
}

function resolveExtensionDir(extensionDir?: string | null): string | null {
  const explicit = extensionDir?.trim();
  if (explicit) {
    const resolved = resolve(explicit);
    return existsSync(resolved) ? resolved : null;
  }

  const candidate = resolve(resolveTablinqAssetPath(["extension"]));
  return existsSync(candidate) ? candidate : null;
}

function detectWindowsBrowserExecutable(): string | null {
  const localAppData = process.env.LOCALAPPDATA?.trim();
  const programFiles = [
    process.env["ProgramFiles(x86)"]?.trim(),
    process.env.ProgramFiles?.trim(),
  ].filter((value): value is string => Boolean(value));

  const chromeCandidates = [
    ...(localAppData
      ? [resolve(localAppData, "Google", "Chrome", "Application", "chrome.exe")]
      : []),
    ...programFiles.map((base) =>
      resolve(base, "Google", "Chrome", "Application", "chrome.exe"),
    ),
  ];
  for (const candidate of chromeCandidates) {
    if (existsSync(candidate)) {
      return candidate;
    }
  }

  const edgeCandidates = [
    ...(localAppData
      ? [
          resolve(
            localAppData,
            "Microsoft",
            "Edge",
            "Application",
            "msedge.exe",
          ),
        ]
      : []),
    ...programFiles.map((base) =>
      resolve(base, "Microsoft", "Edge", "Application", "msedge.exe"),
    ),
  ];
  for (const candidate of edgeCandidates) {
    if (existsSync(candidate)) {
      return candidate;
    }
  }

  for (const commandName of ["chrome.exe", "msedge.exe"]) {
    try {
      const resolved = execFileSync("where", [commandName], {
        encoding: "utf8",
      })
        .split(/\r?\n/)
        .map((value) => value.trim())
        .find(Boolean);
      if (resolved) {
        return resolved;
      }
    } catch {
      continue;
    }
  }

  return null;
}

function inferBrowserKind(browserPath: string): string {
  const fileName = basename(browserPath).toLowerCase();
  if (fileName === "chrome.exe" || fileName === "google-chrome") {
    return "chrome";
  }
  if (fileName === "msedge.exe" || fileName === "microsoft-edge") {
    return "msedge";
  }
  if (fileName.includes("chromium")) {
    return "chromium";
  }
  return "custom";
}

function telemetryKey(capability: string, strategyId: string): string {
  return `${capability}:${strategyId}`;
}

async function captureDomSnapshot(
  page: Page,
  neighborhoods: DomNeighborhoodSnapshot[],
): Promise<string> {
  return page.evaluate((entries) => {
    const excerpt = document.body?.innerHTML?.slice(0, 16_000) ?? "";
    return JSON.stringify(
      {
        title: document.title,
        url: window.location.href,
        neighborhoods: entries,
        body_excerpt: excerpt,
      },
      null,
      2,
    );
  }, neighborhoods);
}

async function captureAriaSnapshot(page: Page): Promise<string> {
  return page.evaluate(() => {
    const isVisible = (element: Element) => {
      if (!(element instanceof HTMLElement)) {
        return false;
      }
      const computed = window.getComputedStyle(element);
      return (
        !element.hidden &&
        element.getAttribute("aria-hidden") !== "true" &&
        computed.display !== "none" &&
        computed.visibility !== "hidden"
      );
    };
    const accessibleName = (element: Element) =>
      (element.getAttribute("aria-label") ?? element.textContent ?? "")
        .replace(/\s+/g, " ")
        .trim();
    const nodes = Array.from(
      document.querySelectorAll(
        [
          "[role]",
          "button",
          "input",
          "textarea",
          "a",
          "[contenteditable='true']",
        ].join(", "),
      ),
    )
      .filter((element) => isVisible(element))
      .slice(0, 120)
      .map((element) => ({
        tag: element.tagName.toLowerCase(),
        role: element.getAttribute("role"),
        name: accessibleName(element),
      }));
    return JSON.stringify(nodes, null, 2);
  });
}

function inferAuthenticated(currentUrl: string): boolean {
  try {
    const url = new URL(currentUrl);
    const localHost =
      url.hostname === "127.0.0.1" || url.hostname === "localhost";
    if (localHost) {
      const path = url.pathname.toLowerCase();
      if (path.includes("login") || path.includes("auth")) {
        return false;
      }
    }
    return (
      url.hostname === "chatgpt.com" ||
      url.hostname === "chat.openai.com" ||
      localHost
    );
  } catch {
    return false;
  }
}

async function safeTitle(page: Page): Promise<string | null> {
  try {
    return await page.title();
  } catch {
    return null;
  }
}

async function resetConversationIdentity(page: Page): Promise<void> {
  await page.evaluate(() => {
    delete document.body.dataset.conversationId;
    delete document.body.dataset.lastMessageId;
    const url = new URL(window.location.href);
    url.searchParams.delete("conversation");
    history.replaceState({}, "", url);
  });
}

function safePageUrl(page: Page): string | null {
  try {
    return page.url();
  } catch {
    return null;
  }
}

function computeStallTimeoutMs(
  requestedTimeoutMs: number | undefined,
  fallbackTimeoutMs: number,
): number {
  const total = requestedTimeoutMs ?? fallbackTimeoutMs;
  return Math.max(
    STALL_DETECTION_MIN_MS,
    Math.min(STALL_DETECTION_MAX_MS, Math.floor(total / 4)),
  );
}

function textRelation(
  emittedText: string,
  observedText: string,
): "same" | "ahead" | "behind" | "mismatch" {
  if (observedText === emittedText) {
    return "same";
  }
  if (observedText.startsWith(emittedText)) {
    return "ahead";
  }
  if (emittedText.startsWith(observedText)) {
    return "behind";
  }
  return "mismatch";
}

interface AskSnapshot {
  text: string;
  state: string;
  conversationId: string | null;
  messageId: string | null;
  currentUrl: string;
}

interface ResearchSnapshot {
  status: "waiting_for_sources" | "running" | "done" | "failed";
  statusMessage: string | null;
  reportText: string;
  conversationId: string | null;
  messageId: string | null;
  currentUrl: string;
}

async function callPageAdapter<T>(
  page: Page,
  methodName:
    | "composerReady"
    | "fillComposer"
    | "clickSubmit"
    | "applyAskConfiguration"
    | "uploadAcknowledged"
    | "readAskSnapshot"
    | "readResearchSnapshot",
  payload?: unknown,
): Promise<T> {
  return page.evaluate(
    ({
      source,
      methodName,
      payload,
    }: {
      source: string;
      methodName: string;
      payload?: unknown;
    }) => {
      const runtime = new Function(
        `${source}\nreturn createTablinqPageAdapterRuntime();`,
      )() as Record<string, (value?: unknown) => unknown>;
      const method = runtime[methodName];
      if (typeof method !== "function") {
        throw new Error(`page adapter method not found: ${methodName}`);
      }
      return method(payload);
    },
    {
      source: loadPageAdapterSource(),
      methodName,
      payload,
    },
  ) as Promise<T>;
}

async function waitForPageAdapterCondition(
  page: Page,
  methodName: "composerReady" | "uploadAcknowledged",
  payload: unknown,
  timeoutMs: number,
): Promise<void> {
  await page.waitForFunction(
    ({
      source,
      methodName,
      payload,
    }: {
      source: string;
      methodName: string;
      payload?: unknown;
    }) => {
      const runtime = new Function(
        `${source}\nreturn createTablinqPageAdapterRuntime();`,
      )() as Record<string, (value?: unknown) => unknown>;
      const method = runtime[methodName];
      return typeof method === "function" ? Boolean(method(payload)) : false;
    },
    {
      source: loadPageAdapterSource(),
      methodName,
      payload,
    },
    {
      timeout: timeoutMs,
    },
  );
}

async function waitForInteractionSurface(page: Page): Promise<void> {
  await page.waitForLoadState("domcontentloaded", {
    timeout: NAVIGATION_TIMEOUT_MS,
  });
  await page
    .waitForLoadState("networkidle", {
      timeout: 2_000,
    })
    .catch(() => undefined);
  await waitForPageAdapterCondition(
    page,
    "composerReady",
    undefined,
    INTERACTION_TIMEOUT_MS,
  );
}

async function fillComposer(page: Page, prompt: string): Promise<void> {
  await waitForInteractionSurface(page);
  await callPageAdapter(page, "fillComposer", { prompt });
}

async function applyAskConfiguration(
  page: Page,
  command: AskExecutionCommand,
  report: CapabilityReportPayload | null,
): Promise<void> {
  if (
    !command.modelClass &&
    !command.effort &&
    (!command.mode || command.mode === "normal")
  ) {
    return;
  }

  if (!report) {
    throw new Error(
      "capability missing: no capability report is available for model or mode selection",
    );
  }

  await waitForInteractionSurface(page);
  await callPageAdapter(page, "applyAskConfiguration", {
    report,
    request: {
      modelClass: command.modelClass ?? undefined,
      effort: command.effort ?? undefined,
      mode: command.mode ?? "normal",
    },
  });
}

async function uploadAttachments(
  page: Page,
  files: string[],
  report: CapabilityReportPayload | null,
): Promise<void> {
  if (files.length === 0) {
    return;
  }

  if (!report?.matrix.upload_control_visible) {
    throw new Error(
      "upload failed: no upload control is available in the current browser session",
    );
  }

  const resolvedFiles = await Promise.all(
    files.map(async (filePath) => {
      const absolutePath = resolve(filePath);
      const fileStat = await stat(absolutePath).catch(() => null);
      if (!fileStat?.isFile()) {
        throw new Error(`upload failed: file not found: ${absolutePath}`);
      }
      return absolutePath;
    }),
  );

  const fileInput = page.locator("input[type='file']").first();
  if ((await fileInput.count()) > 0) {
    await fileInput.setInputFiles(resolvedFiles);
  } else {
    const uploadEntry = page
      .locator(
        [
          "[data-slot='upload-entry']",
          "[data-testid='upload-button']",
          "label[for]",
          "button[aria-label*='file']",
          "button[aria-label*='photo']",
        ].join(", "),
      )
      .first();
    if ((await uploadEntry.count()) === 0) {
      throw new Error("upload failed: upload entry point could not be found");
    }
    const chooserPromise = page.waitForEvent("filechooser", {
      timeout: INTERACTION_TIMEOUT_MS,
    });
    await uploadEntry.click();
    const chooser = await chooserPromise.catch(() => null);
    if (!chooser) {
      throw new Error("upload failed: file chooser did not open");
    }
    await chooser.setFiles(resolvedFiles);
  }

  const expectedNames = resolvedFiles.map((filePath) => basename(filePath));
  await waitForPageAdapterCondition(
    page,
    "uploadAcknowledged",
    {
      names: expectedNames,
    },
    UPLOAD_ACK_TIMEOUT_MS,
  ).catch(() => {
    throw new Error(
      `upload failed: the page did not acknowledge uploaded files: ${expectedNames.join(", ")}`,
    );
  });
}

async function clickSubmit(page: Page): Promise<void> {
  await waitForInteractionSurface(page);
  const clicked = await callPageAdapter<boolean>(page, "clickSubmit");
  if (clicked) {
    return;
  }

  const modifier = process.platform === "darwin" ? "Meta" : "Control";
  await page.keyboard.press(`${modifier}+Enter`).catch(() => undefined);
}

async function readAskSnapshot(page: Page): Promise<AskSnapshot> {
  return callPageAdapter<AskSnapshot>(page, "readAskSnapshot");
}

async function readResearchSnapshot(page: Page): Promise<ResearchSnapshot> {
  return callPageAdapter<ResearchSnapshot>(page, "readResearchSnapshot");
}

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

function isTransientPageError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return (
    message.includes("Execution context was destroyed") ||
    message.includes("Cannot find context") ||
    message.includes("Target page, context or browser has been closed") ||
    message.includes("Frame has been detached") ||
    message.includes("navigation")
  );
}

async function collectArtifacts(
  page: Page,
  downloadsDir: string,
  conversationId: string | null,
  messageId: string | null,
): Promise<ArtifactMetadataSnapshot[]> {
  const handles = await page
    .locator(
      [
        "[data-slot='artifact-link']",
        "[data-testid='artifact-download']",
        "a[download]",
        "a[aria-label*='Download']",
        "button[aria-label*='Download']",
        "a:has-text('Download')",
        "button:has-text('Download')",
        "a:has-text('Export')",
        "button:has-text('Export')",
      ].join(", "),
    )
    .all();
  const artifacts: ArtifactMetadataSnapshot[] = [];

  for (let index = 0; index < handles.length; index += 1) {
    const handle = handles[index]!;
    const artifactReady = await handle.evaluate((element) => {
      if (!(element instanceof HTMLElement)) {
        return false;
      }
      const explicit =
        element.dataset.artifactReady ??
        element.getAttribute("data-artifact-ready");
      if (explicit === "false") {
        return false;
      }
      if (explicit === "true") {
        return true;
      }
      if (element instanceof HTMLAnchorElement) {
        return Boolean(element.hasAttribute("download") || element.href);
      }
      return !(
        element.getAttribute("aria-disabled") === "true" ||
        element.hasAttribute("disabled")
      );
    });
    if (!artifactReady) {
      continue;
    }

    const download = await clickAndCaptureDownload(page, handle).catch(
      (error) => {
        throw new Error(
          `download failed: ${error instanceof Error ? error.message : String(error)}`,
        );
      },
    );
    artifacts.push(
      await persistDownload(
        download,
        downloadsDir,
        conversationId,
        messageId,
        index,
      ),
    );
  }

  return artifacts;
}

async function clickAndCaptureDownload(
  page: Page,
  handle: Locator,
): Promise<Download> {
  const downloadPromise = page.waitForEvent("download", {
    timeout: DOWNLOAD_TIMEOUT_MS,
  });
  await handle.click({ noWaitAfter: true, force: true });
  return downloadPromise;
}

async function persistDownload(
  download: Download,
  downloadsDir: string,
  conversationId: string | null,
  messageId: string | null,
  index: number,
): Promise<ArtifactMetadataSnapshot> {
  const suggestedName =
    download.suggestedFilename() || `artifact-${index + 1}.bin`;
  const storagePath = await chooseUniqueArtifactPath(
    downloadsDir,
    suggestedName,
  );
  await mkdir(dirname(storagePath), { recursive: true });
  await download.saveAs(storagePath);

  const fileBuffer = await readFile(storagePath);
  const fileStat = await stat(storagePath);
  const sha256 = createHash("sha256").update(fileBuffer).digest("hex");
  const artifactId = createHash("sha1")
    .update(
      `${conversationId ?? "no-conversation"}:${messageId ?? "no-message"}:${storagePath}`,
    )
    .digest("hex")
    .slice(0, 16);

  return {
    artifact_id: `artifact-${artifactId}`,
    conversation_id: conversationId,
    message_id: messageId,
    file_name: basename(storagePath),
    content_type: contentTypeForFile(storagePath),
    size_bytes: fileStat.size,
    sha256,
    storage_path: storagePath,
  };
}

export async function chooseUniqueArtifactPath(
  downloadsDir: string,
  fileName: string,
): Promise<string> {
  const absoluteDownloadsDir = resolve(downloadsDir);
  const baseName = basename(fileName);
  let candidate = resolve(absoluteDownloadsDir, baseName);
  if (!(await pathExists(candidate))) {
    return candidate;
  }

  const extension = extname(baseName);
  const stem = extension ? baseName.slice(0, -extension.length) : baseName;
  for (let index = 2; index < 10_000; index += 1) {
    candidate = resolve(absoluteDownloadsDir, `${stem}-${index}${extension}`);
    if (!(await pathExists(candidate))) {
      return candidate;
    }
  }

  throw new Error(
    `download failed: could not allocate a unique path for ${fileName}`,
  );
}

async function pathExists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

function contentTypeForFile(path: string): string {
  switch (extname(path).toLowerCase()) {
    case ".txt":
      return "text/plain; charset=utf-8";
    case ".json":
      return "application/json";
    case ".csv":
      return "text/csv; charset=utf-8";
    case ".html":
      return "text/html; charset=utf-8";
    case ".pdf":
      return "application/pdf";
    default:
      return "application/octet-stream";
  }
}

function workspaceKeyFromUrl(rawUrl: string): string {
  try {
    const url = new URL(rawUrl);
    const params = new URLSearchParams(url.search);
    params.delete("conversation");
    params.delete("scenario");
    const query = params.toString();
    return `${url.origin}${url.pathname}${query ? `?${query}` : ""}`;
  } catch {
    return rawUrl;
  }
}

function matchesTargetWorkspace(pageUrl: string, targetUrl: string): boolean {
  try {
    const page = new URL(pageUrl);
    const target = new URL(targetUrl);
    if (page.origin !== target.origin || page.pathname !== target.pathname) {
      return false;
    }
    return workspaceKeyFromUrl(pageUrl) === workspaceKeyFromUrl(targetUrl);
  } catch {
    return false;
  }
}

function hasUsableCapabilityReport(report: CapabilityReportPayload): boolean {
  return report.matrix.composer_ready || report.matrix.submit_available;
}

function buildTargetPageId(page: Page): string {
  return createHash("sha1")
    .update(
      `${workspaceKeyFromUrl(page.url())}:${page.context().pages().indexOf(page)}`,
    )
    .digest("hex")
    .slice(0, 16);
}

function resolveTablinqAssetPath(
  installedRelativeSegments: string[],
  repoRelativeSegments: string[] = installedRelativeSegments,
): string {
  const candidates: string[] = [];
  const assetRoot = process.env.TABLINQ_ASSET_ROOT;
  if (assetRoot) {
    candidates.push(resolve(assetRoot, ...installedRelativeSegments));
    candidates.push(resolve(assetRoot, ...repoRelativeSegments));
  }

  for (const upward of [
    "..",
    "../..",
    "../../..",
    "../../../..",
    "../../../../..",
  ]) {
    candidates.push(resolve(__dirname, upward, ...installedRelativeSegments));
    candidates.push(resolve(__dirname, upward, ...repoRelativeSegments));
  }

  for (const candidate of candidates) {
    if (existsSync(candidate)) {
      return candidate;
    }
  }

  return resolve(
    assetRoot ?? __dirname,
    ...(assetRoot ? installedRelativeSegments : repoRelativeSegments),
  );
}
