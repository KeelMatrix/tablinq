import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
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

interface ProbeDocumentOptions {
  selectorPackId?: string;
  selectorPackVersion?: string;
  reportedAt?: string;
  preferredStrategies?: Partial<Record<CapabilityKey, string>>;
}

let probeSource: string | undefined;

function loadProbeSource(): string {
  if (!probeSource) {
    probeSource = readFileSync(
      resolve(__dirname, "../../extension/src/probe.runtime.js"),
      "utf8",
    ).replace(/^export\s+function\s+probeDocument/m, "function probeDocument");
  }

  return probeSource;
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

interface CapabilityCacheEntry {
  workspace_key: string;
  saved_at: string;
  ui_signature: string | null;
  preferred_strategies: Partial<Record<CapabilityKey, string>>;
  report: CapabilityReportPayload;
}

interface CapabilityCacheFile {
  version: 1;
  workspaces: Record<string, CapabilityCacheEntry>;
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
  private lastDomNeighborhoods: DomNeighborhoodSnapshot[] = [];
  private readonly capabilityCachePath: string;
  private capabilityCacheLoaded = false;
  private capabilityCache: CapabilityCacheFile = {
    version: 1,
    workspaces: {},
  };

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

  async ask(
    command: AskExecutionCommand,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
  ): Promise<AskResultSnapshot> {
    await this.ensureContext(false, this.currentBrowserPath ?? undefined);
    const page = await this.ensureManagedPage(
      this.resolveConversationTarget(command.conversationId),
    );
    await this.refreshCapabilities(page);
    await applyAskConfiguration(page, command, this.lastCapabilityReport);
    await uploadAttachments(
      page,
      command.files ?? [],
      this.lastCapabilityReport,
    );
    await fillComposer(page, command.prompt);
    await clickSubmit(page);

    const timeoutAt = Date.now() + (command.timeoutMs ?? 15000);
    let emittedText = "";
    let nextSeq = 0;

    while (Date.now() < timeoutAt) {
      let snapshot;
      try {
        snapshot = await readAskSnapshot(page);
      } catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        if (
          message.includes("Execution context was destroyed") ||
          message.includes("Cannot find context")
        ) {
          await page.waitForLoadState("domcontentloaded");
          await sleep(50);
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
        const artifacts = await collectArtifacts(
          page,
          this.downloadsDir,
          snapshot.conversationId,
          snapshot.messageId,
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

      await sleep(50);
    }

    throw new Error("ask timed out waiting for streamed response");
  }

  async research(
    command: ResearchExecutionCommand,
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
  ): Promise<ResearchResultSnapshot> {
    await this.ensureContext(false, this.currentBrowserPath ?? undefined);
    const page = await this.ensureManagedPage(this.currentTargetUrl);
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
    const timeoutAt = Date.now() + (command.timeoutMs ?? 30_000);

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
        const artifacts = await collectArtifacts(
          page,
          this.downloadsDir,
          snapshot.conversationId,
          snapshot.messageId,
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

      await sleep(75);
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

    if (page.url() !== targetUrl) {
      await page.goto(targetUrl, { waitUntil: "domcontentloaded" });
    } else {
      await page.waitForLoadState("domcontentloaded");
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
    this.context = await chromium.launchPersistentContext(this.profileDir, {
      acceptDownloads: true,
      channel: launch.channel,
      executablePath: launch.executablePath,
      downloadsPath: this.downloadsDir,
      headless,
    });
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
      await page.goto(this.currentTargetUrl, { waitUntil: "domcontentloaded" });
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

    try {
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
            selectorPackId: "chatgpt-default",
            selectorPackVersion: "v1",
            preferredStrategies,
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
      this.persistCapabilitySnapshot(page.url(), message);
    } catch {
      this.extensionConnected = false;
      this.lastCapabilityReport = null;
      this.lastDomNeighborhoods = [];
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
      ) as CapabilityCacheFile;
      if (parsed?.version !== 1 || !parsed.workspaces) {
        return;
      }
      this.capabilityCache = parsed;
      for (const entry of Object.values(parsed.workspaces)) {
        if (entry.ui_signature) {
          this.strategyTelemetry.set(
            entry.ui_signature,
            entry.preferred_strategies ?? {},
          );
        }
      }
    } catch {
      this.capabilityCache = {
        version: 1,
        workspaces: {},
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

function inferAuthenticated(currentUrl: string): boolean {
  try {
    const url = new URL(currentUrl);
    return (
      url.hostname === "chatgpt.com" ||
      url.hostname === "chat.openai.com" ||
      url.hostname === "127.0.0.1" ||
      url.hostname === "localhost"
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

async function fillComposer(page: Page, prompt: string): Promise<void> {
  await page.evaluate((value: string) => {
    const composer = document.querySelector(
      "[data-slot='composer'], textarea, [role='textbox'], [contenteditable='true']",
    );
    if (!(composer instanceof HTMLElement)) {
      throw new Error("composer not found");
    }

    composer.focus();
    if (
      composer instanceof HTMLTextAreaElement ||
      composer instanceof HTMLInputElement
    ) {
      composer.value = value;
      composer.dispatchEvent(new Event("input", { bubbles: true }));
      return;
    }

    composer.textContent = value;
    composer.dispatchEvent(
      new InputEvent("input", { bubbles: true, data: value }),
    );
  }, prompt);
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

  await page.evaluate(
    ({
      source,
      report,
      request,
    }: {
      source: string;
      report: CapabilityReportPayload;
      request: {
        mode: "normal" | "search" | "research";
        modelClass?: "instant" | "thinking";
        effort?: "none" | "standard" | "extended";
      };
    }) => {
      const applyInPage = new Function(
        `${source}\nreturn applyAskConfigurationInPage;`,
      )() as (
        report: CapabilityReportPayload,
        request: {
          mode: "normal" | "search" | "research";
          modelClass?: "instant" | "thinking";
          effort?: "none" | "standard" | "extended";
        },
      ) => void;
      applyInPage(report, request);
    },
    {
      source: applyAskConfigurationSource,
      report,
      request: {
        modelClass: command.modelClass ?? undefined,
        effort: command.effort ?? undefined,
        mode: command.mode ?? "normal",
      },
    },
  );
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
      .locator("[data-slot='upload-entry'], label[for], [aria-label*='file']")
      .first();
    if ((await uploadEntry.count()) === 0) {
      throw new Error("upload failed: upload entry point could not be found");
    }
    const chooserPromise = page.waitForEvent("filechooser", { timeout: 2_000 });
    await uploadEntry.click();
    const chooser = await chooserPromise.catch(() => null);
    if (!chooser) {
      throw new Error("upload failed: file chooser did not open");
    }
    await chooser.setFiles(resolvedFiles);
  }

  const expectedNames = resolvedFiles.map((filePath) => basename(filePath));
  await page
    .waitForFunction(
      (names: string[]) => {
        const uploaded = new Set(
          Array.from(
            document.querySelectorAll(
              "[data-slot='uploaded-file'], [data-slot='upload-entry-name']",
            ),
          )
            .map((element) => element.textContent?.trim() ?? "")
            .filter((value) => value.length > 0),
        );
        if (uploaded.size === 0) {
          const fromDataset = document.body.dataset.uploadedFiles
            ?.split("|")
            .map((value) => value.trim())
            .filter(Boolean);
          if (fromDataset?.length) {
            return names.every((name) => fromDataset.includes(name));
          }
        }
        return names.every((name) => uploaded.has(name));
      },
      expectedNames,
      { timeout: 5_000 },
    )
    .catch(() => {
      throw new Error(
        `upload failed: the page did not acknowledge uploaded files: ${expectedNames.join(", ")}`,
      );
    });
}

async function clickSubmit(page: Page): Promise<void> {
  await page.evaluate(() => {
    const submit = document.querySelector(
      "[data-action='submit'], [aria-label='Submit prompt'], button[type='submit']",
    );
    if (!(submit instanceof HTMLElement)) {
      throw new Error("submit action not found");
    }
    submit.click();
  });
}

async function readAskSnapshot(page: Page): Promise<{
  text: string;
  state: string;
  conversationId: string | null;
  messageId: string | null;
  currentUrl: string;
}> {
  return page.evaluate(() => {
    const article = document.querySelector(
      "[data-slot='assistant-message'], .message",
    );
    const textNode =
      document.querySelector("[data-slot='assistant-message'] p") ??
      document.querySelector(".message p");
    const text = textNode?.textContent?.trim() ?? "";
    const state =
      article?.getAttribute("data-stream-state") ??
      document.body.dataset.streamState ??
      "idle";
    const conversationId =
      document.body.dataset.conversationId ??
      new URL(window.location.href).searchParams.get("conversation");
    const messageId =
      article?.getAttribute("data-message-id") ??
      document.body.dataset.lastMessageId ??
      null;

    return {
      text,
      state,
      conversationId,
      messageId,
      currentUrl: window.location.href,
    };
  });
}

async function readResearchSnapshot(page: Page): Promise<{
  status: "waiting_for_sources" | "running" | "done" | "failed";
  statusMessage: string | null;
  reportText: string;
  conversationId: string | null;
  messageId: string | null;
  currentUrl: string;
}> {
  return page.evaluate(() => {
    const reportNode =
      document.querySelector("[data-slot='research-report'] p") ??
      document.querySelector("[data-slot='assistant-message'] p") ??
      document.querySelector(".message p");
    const status =
      (document.body.dataset.researchState as
        | "waiting_for_sources"
        | "running"
        | "done"
        | "failed"
        | undefined) ?? "running";
    return {
      status,
      statusMessage: document.body.dataset.researchStatusMessage ?? null,
      reportText: reportNode?.textContent?.trim() ?? "",
      conversationId:
        document.body.dataset.conversationId ??
        new URL(window.location.href).searchParams.get("conversation"),
      messageId: document.body.dataset.lastMessageId ?? null,
      currentUrl: window.location.href,
    };
  });
}

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function collectArtifacts(
  page: Page,
  downloadsDir: string,
  conversationId: string | null,
  messageId: string | null,
): Promise<ArtifactMetadataSnapshot[]> {
  const handles = await page
    .locator("[data-slot='artifact-link'], a[download]")
    .all();
  const artifacts: ArtifactMetadataSnapshot[] = [];

  for (let index = 0; index < handles.length; index += 1) {
    const handle = handles[index]!;
    const artifactReady = await handle.evaluate((element) => {
      if (!(element instanceof HTMLElement)) {
        return false;
      }
      return (
        element.dataset.artifactReady === "true" ||
        element.getAttribute("data-artifact-ready") === "true"
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
  const downloadPromise = page.waitForEvent("download", { timeout: 5_000 });
  await handle.click({ noWaitAfter: true });
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

const applyAskConfigurationSource = String.raw`
function applyAskConfigurationInPage(report, request) {
  const normalizeText = (value) =>
    (value ?? "").replace(/\s+/g, " ").trim().toLowerCase();

  const hasToken = (value, tokens) =>
    tokens.some((token) =>
      new RegExp(
        '(^|[^a-z0-9])' +
          token.replace(/[.*+?^$()|[\]{}\\]/g, '\\$&') +
          '([^a-z0-9]|$)',
      ).test(value),
    );

  const isVisible = (element) => {
    if (!(element instanceof HTMLElement)) {
      return false;
    }
    if (element.hidden || element.getAttribute("aria-hidden") === "true") {
      return false;
    }
    const computed = window.getComputedStyle(element);
    return (
      computed.display !== "none" &&
      computed.visibility !== "hidden" &&
      computed.opacity !== "0"
    );
  };

  const accessibleName = (element) => {
    if (!(element instanceof HTMLElement)) {
      return "";
    }
    const ariaLabel = element.getAttribute("aria-label");
    if (ariaLabel) {
      return normalizeText(ariaLabel);
    }
    if (
      element instanceof HTMLInputElement ||
      element instanceof HTMLTextAreaElement
    ) {
      if (element.labels?.length) {
        return normalizeText(
          Array.from(element.labels)
            .map((label) => label.textContent ?? "")
            .join(" "),
        );
      }
      if (element.placeholder) {
        return normalizeText(element.placeholder);
      }
    }
    return normalizeText(element.textContent);
  };

  const actionableElements = () =>
    Array.from(
      document.querySelectorAll(
        [
          "button",
          "[role='button']",
          "[role='tab']",
          "[role='switch']",
          "[role='option']",
          "[aria-haspopup]",
          "label[for]",
        ].join(", "),
      ),
    ).filter(isVisible);

  const isSelected = (element) =>
    element.getAttribute("aria-pressed") === "true" ||
    element.getAttribute("aria-checked") === "true" ||
    element.getAttribute("aria-selected") === "true" ||
    element.dataset.selected === "true" ||
    element.dataset.active === "true";

  const controlStateDetail = (aliases, requested) => {
    const entries = Object.entries(aliases ?? {})
      .map(([key, labels]) => key + "=[" + labels.join(", ") + "]")
      .join("; ");
    if (!entries) {
      return "requested=" + requested + "; discovered=none";
    }
    return "requested=" + requested + "; discovered=" + entries;
  };

  const throwCapabilityMissing = (capability, requested, aliases) => {
    throw new Error(
      "capability missing: " +
        capability +
        "; " +
        controlStateDetail(aliases, requested),
    );
  };

  const throwUiDrift = (detail) => {
    throw new Error("ui drift: " + detail);
  };

  const clickControl = (element) => {
    if (element.tagName.toLowerCase() === "label") {
      const inputId = element.getAttribute("for");
      if (inputId) {
        const input = document.getElementById(inputId);
        if (input instanceof HTMLElement) {
          input.click();
          return;
        }
      }
    }
    element.click();
  };

  const findBySelector = (selector) => {
    for (const element of Array.from(document.querySelectorAll(selector))) {
      if (isVisible(element)) {
        return element;
      }
    }
    return null;
  };

  const findByAliases = (aliases, fallbackTokens) => {
    const normalizedAliases = aliases.map((alias) => normalizeText(alias));
    for (const element of actionableElements()) {
      const name = accessibleName(element);
      if (!name) {
        continue;
      }
      if (
        normalizedAliases.some(
          (alias) =>
            name === alias || name.includes(alias) || alias.includes(name),
        )
      ) {
        return element;
      }
    }
    for (const element of actionableElements()) {
      const name = accessibleName(element);
      if (hasToken(name, fallbackTokens)) {
        return element;
      }
    }
    return null;
  };

  const ensureMode = (mode) => {
    if (mode === "normal") {
      return;
    }
    const capabilityVisible =
      mode === "search"
        ? report.matrix.search_mode_visible
        : report.matrix.research_mode_visible;
    if (!capabilityVisible) {
      throwCapabilityMissing(mode + " mode", mode, report.aliases.modes);
    }

    const fallbackSelector =
      mode === "search" ? "[data-mode='search']" : "[data-mode='research']";
    const fallbackTokens =
      mode === "search" ? ["search", "browse", "web"] : ["research", "deep"];
    const control =
      findBySelector(fallbackSelector) ??
      findByAliases(report.aliases.modes[mode] ?? [], fallbackTokens);
    if (!control) {
      throwUiDrift("could not find " + mode + " mode control after probing it");
    }
    if (!isSelected(control)) {
      clickControl(control);
    }
  };

  const ensureModelClass = (modelClass) => {
    if (!report.matrix.model_picker_visible) {
      throwCapabilityMissing(
        "model picker",
        modelClass,
        report.aliases.model_classes,
      );
    }
    const aliases = report.aliases.model_classes[modelClass] ?? [];
    if (aliases.length === 0) {
      throwCapabilityMissing(
        "model class",
        modelClass,
        report.aliases.model_classes,
      );
    }
    const control =
      findBySelector("[data-model='" + modelClass + "']") ??
      findByAliases(aliases, [modelClass]);
    if (!control) {
      throwUiDrift(
        "could not find a control for model=" +
          modelClass +
          " after alias discovery",
      );
    }
    if (!isSelected(control)) {
      clickControl(control);
    }
  };

  const ensureEffort = (effort) => {
    if (effort === "none") {
      return;
    }
    if (!report.matrix.thinking_effort_visible) {
      throwCapabilityMissing(
        "thinking effort",
        effort,
        report.aliases.efforts,
      );
    }
    const aliases = report.aliases.efforts[effort] ?? [];
    if (aliases.length === 0) {
      throwCapabilityMissing(
        "thinking effort",
        effort,
        report.aliases.efforts,
      );
    }
    const control =
      findBySelector("[data-effort='" + effort + "']") ??
      findByAliases(aliases, [effort, "reasoning", "effort"]);
    if (!control) {
      throwUiDrift(
        "could not find a control for effort=" +
          effort +
          " after alias discovery",
      );
    }
    if (!isSelected(control)) {
      clickControl(control);
    }
  };

  if (request.modelClass) {
    ensureModelClass(request.modelClass);
  }
  if (request.effort) {
    ensureEffort(request.effort);
  }
  if (request.mode) {
    ensureMode(request.mode);
  }
}
`;
