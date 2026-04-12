import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { chromium, type BrowserContext, type Page } from "playwright-core";

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
}

export interface AskResultSnapshot {
  job_id: string;
  conversation_id: string;
  message_id: string;
  output_text: string;
  completed_at: string;
  current_url: string;
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

  constructor(options: BrowserAdapterOptions) {
    this.profileDir = resolve(options.profileDir);
    this.downloadsDir = resolve(options.downloadsDir);
    this.defaultTargetUrl = options.defaultTargetUrl;
    this.defaultBrowserPath = options.defaultBrowserPath ?? null;
    this.currentTargetUrl = options.defaultTargetUrl;
    this.currentBrowserPath = options.defaultBrowserPath ?? null;
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
          completed_at: new Date().toISOString(),
          current_url: snapshot.currentUrl,
        };
      }

      await sleep(50);
    }

    throw new Error("ask timed out waiting for streamed response");
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
      : {};

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
    } catch {
      this.extensionConnected = false;
      this.lastCapabilityReport = null;
      this.lastDomNeighborhoods = [];
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

async function sleep(ms: number): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, ms));
}
