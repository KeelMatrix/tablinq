import { mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { execFileSync } from "node:child_process";
import { chromium, type BrowserContext, type Page } from "playwright-core";

export interface BrowserOpenCommand {
  targetUrl: string;
  browserPath?: string | null;
  headless?: boolean;
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

    if (!this.context) {
      await mkdir(this.profileDir, { recursive: true });
      await mkdir(this.downloadsDir, { recursive: true });
      const launch = resolveBrowserLaunchConfig(
        process.platform,
        browserPath ?? undefined,
      );
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

    await this.ensureManagedPage(targetUrl);
    return this.health();
  }

  async health(): Promise<BrowserStatusSnapshot> {
    await this.recoverManagedPage();
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
      page_title: running ? await safeTitle(page!) : null,
      page_ready: running ? !page!.isClosed() : false,
      authenticated: running ? inferAuthenticated(page!.url()) : false,
      reload_count: this.reloadCount,
      screenshot_path: this.lastScreenshotPath,
      browser_path: this.currentBrowserPath,
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
    return this.health();
  }

  async shutdown(): Promise<BrowserStatusSnapshot> {
    return this.close();
  }

  getManagedPageForTest(): Page | undefined {
    return this.managedPage;
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
