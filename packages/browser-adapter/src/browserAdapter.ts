import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { mkdir, readFile, stat } from "node:fs/promises";
import { basename, dirname, extname, resolve } from "node:path";
import {
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
const INTERACTION_TIMEOUT_MS = 7_500;
const UPLOAD_ACK_TIMEOUT_MS = 10_000;
const DOWNLOAD_TIMEOUT_MS = 10_000;
const SNAPSHOT_POLL_INTERVAL_MS = 75;
const STABLE_DONE_POLLS = 3;

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

export interface BrowserAdapterOptions {
  profileDir: string;
  downloadsDir: string;
  defaultTargetUrl: string;
  defaultBrowserPath?: string | null;
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
}

export interface BrowserLaunchConfig {
  browserKind: string;
  channel?: "msedge";
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

export class BrowserAdapter {
  private context: BrowserContext | undefined;
  private managedPage: Page | undefined;
  private readonly profileDir: string;
  private readonly downloadsDir: string;
  private readonly defaultTargetUrl: string;
  private readonly defaultBrowserPath: string | null;
  private currentTargetUrl: string;
  private currentBrowserPath: string | null;
  private reloadCount = 0;
  private lastScreenshotPath: string | null = null;
  private readonly observedPages = new WeakSet<Page>();
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
      this.extensionConnected = false;
      this.lastCapabilityReport = null;
      this.lastDomNeighborhoods = [];
      this.lastUiSignature = null;
    }

    return {
      running,
      browser_kind: resolveBrowserLaunchConfig(
        process.platform,
        this.currentBrowserPath ?? undefined,
      ).browserKind,
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
    };
  }

  async snapshot(): Promise<BrowserStatusSnapshot> {
    const page = this.managedPage;
    const running = Boolean(this.context && page && !page.isClosed());

    return {
      running,
      browser_kind: resolveBrowserLaunchConfig(
        process.platform,
        this.currentBrowserPath ?? undefined,
      ).browserKind,
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
    };
  }

  async screenshot(path: string): Promise<BrowserStatusSnapshot> {
    await this.recoverManagedPage();
    if (!this.managedPage || this.managedPage.isClosed()) {
      throw new Error("managed page is not open");
    }

    const screenshotPath = resolve(path);
    await mkdir(dirname(screenshotPath), { recursive: true });
    await this.managedPage.screenshot({ fullPage: true, path: screenshotPath });
    this.lastScreenshotPath = screenshotPath;
    return this.health();
  }

  async close(): Promise<BrowserStatusSnapshot> {
    if (this.context) {
      await this.context.close();
    }
    this.context = undefined;
    this.managedPage = undefined;
    this.extensionConnected = false;
    this.lastCapabilityReport = null;
    this.lastDomNeighborhoods = [];
    this.lastUiSignature = null;
    return this.health();
  }

  async shutdown(): Promise<BrowserStatusSnapshot> {
    return this.close();
  }

  getManagedPageForTest(): Page | undefined {
    return this.managedPage;
  }

  getLastDomNeighborhoodsForTest(): DomNeighborhoodSnapshot[] {
    return [...this.lastDomNeighborhoods];
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
  ): Promise<AskResultSnapshot> {
    await this.ensureContext(false, this.currentBrowserPath ?? undefined);
    const page = await this.ensureManagedPage(
      this.resolveConversationTarget(command.conversationId),
    );
    if (!inferAuthenticated(page.url())) {
      throw new Error(
        "auth required: managed browser session is not authenticated",
      );
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

    const timeoutAt = Date.now() + (command.timeoutMs ?? ASK_TIMEOUT_MS);
    let emittedText = "";
    let nextSeq = 0;
    let doneStreak = 0;

    while (Date.now() < timeoutAt) {
      let snapshot;
      try {
        snapshot = await readAskSnapshot(page);
      } catch (error) {
        if (isTransientPageError(error)) {
          await page.waitForLoadState("domcontentloaded", {
            timeout: PROBE_TIMEOUT_MS,
          });
          await sleep(SNAPSHOT_POLL_INTERVAL_MS);
          continue;
        }
        throw error;
      }
      if (
        snapshot.text.length > emittedText.length &&
        snapshot.text.startsWith(emittedText)
      ) {
        const delta = snapshot.text.slice(emittedText.length);
        emittedText = snapshot.text;
        await onChunk({
          job_id: command.jobId,
          conversation_id: snapshot.conversationId,
          message_id: snapshot.messageId,
          seq: nextSeq,
          kind: "delta",
          text: delta,
        });
        nextSeq += 1;
      }

      if (snapshot.state === "done") {
        doneStreak += 1;
      } else {
        doneStreak = 0;
      }

      if (snapshot.state === "done" && doneStreak >= STABLE_DONE_POLLS) {
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
          seq: nextSeq,
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

      await sleep(SNAPSHOT_POLL_INTERVAL_MS);
    }

    throw new Error("ask timed out waiting for streamed response");
  }

  async research(
    command: ResearchExecutionCommand,
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
  ): Promise<ResearchResultSnapshot> {
    await this.ensureContext(false, this.currentBrowserPath ?? undefined);
    const page = await this.ensureManagedPage(this.currentTargetUrl);
    if (!inferAuthenticated(page.url())) {
      throw new Error(
        "auth required: managed browser session is not authenticated",
      );
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

    let emittedText = "";
    let lastStatus = "starting";
    let lastMessage = "starting research";
    const timeoutAt = Date.now() + (command.timeoutMs ?? RESEARCH_TIMEOUT_MS);
    let doneStreak = 0;

    await onUpdate({
      status: "starting",
      message: "starting research",
      conversation_id: null,
      message_id: null,
      current_url: page.url(),
    });

    while (Date.now() < timeoutAt) {
      const snapshot = await readResearchSnapshot(page);
      if (
        snapshot.status !== lastStatus ||
        snapshot.statusMessage !== lastMessage
      ) {
        lastStatus = snapshot.status;
        lastMessage = snapshot.statusMessage ?? "";
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
        snapshot.reportText.length > emittedText.length &&
        snapshot.reportText.startsWith(emittedText)
      ) {
        const delta = snapshot.reportText.slice(emittedText.length);
        emittedText = snapshot.reportText;
        await onUpdate({
          status: "running",
          message: snapshot.statusMessage ?? undefined,
          conversation_id: snapshot.conversationId,
          message_id: snapshot.messageId,
          current_url: snapshot.currentUrl,
          chunk_kind: "delta",
          chunk_text: delta,
        });
      }

      if (snapshot.status === "failed") {
        throw new Error(
          `research job failed: ${snapshot.statusMessage ?? "research failed"}`,
        );
      }

      if (snapshot.status === "done") {
        doneStreak += 1;
      } else {
        doneStreak = 0;
      }

      if (snapshot.status === "done" && doneStreak >= STABLE_DONE_POLLS) {
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

      await sleep(SNAPSHOT_POLL_INTERVAL_MS);
    }

    throw new Error("research job failed: timed out waiting for completion");
  }

  private async ensureManagedPage(targetUrl: string): Promise<Page> {
    await this.recoverManagedPage();
    let page = this.managedPage;

    if (!page || page.isClosed()) {
      page = await this.context!.newPage();
      this.attachPage(page);
      this.managedPage = page;
    }

    try {
      if (page.url() !== targetUrl) {
        await page.goto(targetUrl, {
          waitUntil: "domcontentloaded",
          timeout: NAVIGATION_TIMEOUT_MS,
        });
      } else {
        await page.waitForLoadState("domcontentloaded", {
          timeout: NAVIGATION_TIMEOUT_MS,
        });
      }
    } catch (error) {
      if (!isTransientPageError(error)) {
        throw error;
      }

      const replacement = await this.context!.newPage();
      this.attachPage(replacement);
      this.managedPage = replacement;
      page = replacement;
      await page.goto(targetUrl, {
        waitUntil: "domcontentloaded",
        timeout: NAVIGATION_TIMEOUT_MS,
      });
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

    this.hydrateCapabilityCache();
    await mkdir(this.profileDir, { recursive: true });
    await mkdir(this.downloadsDir, { recursive: true });
    const launch = resolveBrowserLaunchConfig(process.platform, browserPath);
    try {
      this.context = await chromium.launchPersistentContext(this.profileDir, {
        acceptDownloads: true,
        channel: launch.channel,
        executablePath: launch.executablePath,
        downloadsPath: this.downloadsDir,
        headless,
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
    this.context.on("page", (page) => {
      this.attachPage(page);
    });
    for (const page of this.context.pages()) {
      this.attachPage(page);
    }
  }

  private resolveConversationTarget(conversationId?: string | null): string {
    if (!conversationId) {
      return this.currentTargetUrl;
    }

    const target = new URL(this.currentTargetUrl);
    target.searchParams.set("conversation", conversationId);
    return target.toString();
  }

  private async recoverManagedPage(): Promise<void> {
    if (!this.context) {
      return;
    }

    if (this.managedPage && !this.managedPage.isClosed()) {
      return;
    }

    const existing = this.context.pages().find((page) => !page.isClosed());
    if (existing) {
      this.attachPage(existing);
      this.managedPage = existing;
      return;
    }

    if (this.currentTargetUrl) {
      const page = await this.context.newPage();
      this.attachPage(page);
      this.managedPage = page;
      await page.goto(this.currentTargetUrl, {
        waitUntil: "domcontentloaded",
        timeout: NAVIGATION_TIMEOUT_MS,
      });
    }
  }

  private attachPage(page: Page): void {
    if (this.observedPages.has(page)) {
      this.managedPage = page;
      return;
    }
    this.observedPages.add(page);

    const mainFrame = page.mainFrame();
    let seenInitialNavigation = false;
    page.on("framenavigated", (frame) => {
      if (frame !== mainFrame) {
        return;
      }
      if (seenInitialNavigation) {
        this.reloadCount += 1;
      } else {
        seenInitialNavigation = true;
      }
    });
    page.on("close", () => {
      if (this.managedPage === page) {
        this.managedPage = undefined;
      }
    });
    page.on("crash", () => {
      if (this.managedPage === page) {
        this.managedPage = undefined;
      }
    });
    this.managedPage = page;
  }

  private async refreshCapabilities(page: Page): Promise<void> {
    if (page.isClosed()) {
      this.extensionConnected = false;
      this.lastCapabilityReport = null;
      this.lastDomNeighborhoods = [];
      this.lastUiSignature = null;
      return;
    }

    const preferredStrategies = this.lastUiSignature
      ? (this.strategyTelemetry.get(this.lastUiSignature) ?? {})
      : (this.capabilityCache.workspaces[workspaceKeyFromUrl(page.url())]
          ?.preferred_strategies ?? {});
    const selectorPack = this.ensureSelectorPackLoaded();

    for (let attempt = 0; attempt < 3; attempt += 1) {
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
        this.extensionConnected = true;
        this.lastCapabilityReport = message.report;
        this.lastUiSignature = message.runtime.ui_signature;
        this.lastDomNeighborhoods = message.runtime.dom_neighborhoods;
        this.strategyTelemetry.set(
          message.runtime.ui_signature,
          message.runtime.strategy_matches,
        );
        this.recordStrategyStats(message.runtime.strategy_matches);
        this.persistCapabilitySnapshot(page.url(), message);
        return;
      } catch (error) {
        if (attempt < 2 && isTransientPageError(error)) {
          await sleep(150 * (attempt + 1));
          continue;
        }
        this.extensionConnected = false;
        this.lastCapabilityReport = null;
        this.lastDomNeighborhoods = [];
        this.lastUiSignature = null;
        return;
      }
    }
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
      browserKind: "custom",
      executablePath: browserPath,
    };
  }

  if (platform === "win32") {
    return {
      browserKind: "msedge",
      channel: "msedge",
    };
  }

  const detected = detectBrowserExecutable(platform);
  if (detected) {
    return {
      browserKind: "chromium",
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
  if (platform === "win32") {
    return { detected: true, path: null };
  }
  const detected = detectBrowserExecutable(platform);
  return {
    detected: Boolean(detected),
    path: detected,
  };
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
