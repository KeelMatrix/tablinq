import { mkdir } from "node:fs/promises";
import {
  type Browser,
  chromium,
  type BrowserContext,
  type Page,
} from "playwright-core";

import {
  hasUsableCapabilityReport,
  inferAuthenticated,
  safePageUrl,
} from "../capabilityProbe";
import { isTransientPageError } from "../pageRuntime";
import { inferBrowserKind, resolveBrowserLaunchConfig } from "../launch";
import { NAVIGATION_TIMEOUT_MS } from "../timing";
import {
  matchesTargetLocation,
  matchesTargetWorkspace,
  uniquePages,
} from "../urls";
import type { BrowserPageRegistry, TargetPageState } from "./pageRegistry";

export interface BrowserSessionControllerOptions {
  profileDir: string;
  downloadsDir: string;
  defaultTargetUrl: string;
  defaultBrowserPath: string | null;
  extensionDir: string | null;
  connectionMode: "attach_first" | "managed_fallback" | "managed_only";
  debugEndpoint: string | null;
  autoOpenTargetTab: boolean;
}

export interface BrowserSessionControllerCallbacks {
  refreshCapabilities(page: Page): Promise<TargetPageState | undefined>;
  applyPageStateToActiveTarget(state: TargetPageState | undefined): void;
  clearActiveCapabilityState(): void;
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

export class BrowserSessionController {
  private browser: Browser | undefined;
  private context: BrowserContext | undefined;
  private managedPage: Page | undefined;
  private currentTargetUrl: string;
  private currentBrowserPath: string | null;
  private currentHeadless = false;
  private reloadCount = 0;
  private targetPageId: string | null = null;
  private targetPageCreatedByAdapter = false;
  private browserOwner: "external" | "managed" = "managed";
  private cdpAttached = false;

  constructor(
    private readonly options: BrowserSessionControllerOptions,
    private readonly pageRegistry: BrowserPageRegistry,
    private readonly callbacks: BrowserSessionControllerCallbacks,
  ) {
    this.currentTargetUrl = options.defaultTargetUrl;
    this.currentBrowserPath = options.defaultBrowserPath;
  }

  get activePage(): Page | undefined {
    return this.managedPage;
  }

  get activePageId(): string | null {
    return this.targetPageId;
  }

  get activeReloadCount(): number {
    return this.reloadCount;
  }

  get activeBrowserOwner(): "external" | "managed" {
    return this.browserOwner;
  }

  get attachedOverCdp(): boolean {
    return this.cdpAttached;
  }

  get targetUrl(): string {
    return this.currentTargetUrl;
  }

  set targetUrl(value: string) {
    this.currentTargetUrl = value;
  }

  get browserPath(): string | null {
    return this.currentBrowserPath;
  }

  get headless(): boolean {
    return this.currentHeadless;
  }

  configureOpen(targetUrl: string, browserPath: string | null): void {
    this.currentTargetUrl = targetUrl;
    this.currentBrowserPath = browserPath;
  }

  statusBrowserKind(): string {
    if (this.currentBrowserPath) {
      return inferBrowserKind(this.currentBrowserPath);
    }
    return process.platform === "win32" ? "msedge" : "chromium";
  }

  activeWorkspaceKey(): string | null {
    return (
      this.pageRegistry.activePageState(this.targetPageId)?.workspace_key ??
      null
    );
  }

  isManagedPage(page: Page): boolean {
    return page === this.managedPage;
  }

  async ensureContext(headless: boolean, browserPath?: string): Promise<void> {
    if (this.context) {
      return;
    }

    this.currentHeadless = headless;
    await mkdir(this.options.profileDir, { recursive: true });
    await mkdir(this.options.downloadsDir, { recursive: true });
    const shouldTryAttach =
      this.options.connectionMode === "attach_first" ||
      this.options.connectionMode === "managed_fallback";
    let attachError: Error | null = null;

    if (shouldTryAttach) {
      try {
        await this.attachToExistingBrowser();
        return;
      } catch (error) {
        attachError = error instanceof Error ? error : new Error(String(error));
        if (this.options.connectionMode === "attach_first") {
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

  async ensureManagedPage(
    targetUrl: string,
    forceReload = false,
  ): Promise<Page> {
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
      await this.navigateTargetPage(page, targetUrl, forceReload);
    } catch (error) {
      if (!isRecoverableTargetPageError(error)) {
        throw error;
      }

      await this.discardAttachedTargetPage(page);
      page = await this.createTargetPage(targetUrl);
      await this.navigateTargetPage(page, targetUrl, forceReload);
    }

    if (this.cdpAttached) {
      await this.cleanupStaleAttachedTargetPages(targetUrl, page);
    }

    return page;
  }

  async recoverManagedPage(): Promise<void> {
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

    if (this.currentTargetUrl && this.options.autoOpenTargetTab) {
      const page = await this.createTargetPage(this.currentTargetUrl);
      await this.navigateTargetPage(page, this.currentTargetUrl);
    }
  }

  activateRecoveredPage(page: Page): void {
    this.context = page.context();
    this.observeContext(this.context);
    this.attachPage(page, true);
    this.targetPageCreatedByAdapter = false;
  }

  async close(): Promise<void> {
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
      this.options.connectionMode === "managed_only" ? "managed" : "external";
    this.cdpAttached = false;
    this.pageRegistry.clearPageStates();
    this.callbacks.clearActiveCapabilityState();
  }

  private attachPage(page: Page, activate = false): void {
    if (this.pageRegistry.hasObservedPage(page)) {
      this.pageRegistry.updatePageLocationState(page, this.cdpAttached);
      if (activate) {
        this.activateTargetPage(page, this.targetPageCreatedByAdapter);
      }
      return;
    }
    this.pageRegistry.markPageObserved(page);
    const pageState = this.pageRegistry.updatePageLocationState(
      page,
      this.cdpAttached,
    );
    const pageId = pageState.page_id;

    const mainFrame = page.mainFrame();
    let seenInitialNavigation = Boolean(
      page.url() && page.url() !== "about:blank",
    );
    page.on("framenavigated", (frame) => {
      if (frame !== mainFrame) {
        return;
      }
      const state = this.pageRegistry.updatePageLocationState(
        page,
        this.cdpAttached,
      );
      if (!page.isClosed() && this.managedPage === page) {
        this.currentTargetUrl = page.url();
        this.targetPageId = state.page_id;
        this.callbacks.applyPageStateToActiveTarget(state);
      }
      if (seenInitialNavigation) {
        this.reloadCount += 1;
      } else {
        seenInitialNavigation = true;
      }
    });
    page.on("close", () => {
      this.pageRegistry.deletePageState(pageId);
      if (this.managedPage === page) {
        this.managedPage = undefined;
        this.targetPageId = null;
        this.targetPageCreatedByAdapter = false;
        this.callbacks.clearActiveCapabilityState();
      }
    });
    page.on("crash", () => {
      this.pageRegistry.deletePageState(pageId);
      if (this.managedPage === page) {
        this.managedPage = undefined;
        this.targetPageId = null;
        this.targetPageCreatedByAdapter = false;
        this.callbacks.clearActiveCapabilityState();
      }
    });
    if (activate) {
      this.activateTargetPage(page, this.targetPageCreatedByAdapter);
    }
  }

  private activateTargetPage(page: Page, createdByAdapter: boolean): void {
    const state = this.pageRegistry.updatePageLocationState(
      page,
      this.cdpAttached,
    );
    this.managedPage = page;
    this.targetPageId = state.page_id;
    this.targetPageCreatedByAdapter = createdByAdapter;
    this.callbacks.applyPageStateToActiveTarget(state);
  }

  private async attachToExistingBrowser(): Promise<void> {
    const endpoint = this.options.debugEndpoint?.trim();
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
      this.pageRegistry.clearPageStates();
      this.callbacks.clearActiveCapabilityState();
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
      !headless && this.options.extensionDir
        ? [
            `--disable-extensions-except=${this.options.extensionDir}`,
            `--load-extension=${this.options.extensionDir}`,
          ]
        : [];
    try {
      this.context = await chromium.launchPersistentContext(
        this.options.profileDir,
        {
          acceptDownloads: true,
          channel: launch.channel,
          executablePath: launch.executablePath,
          downloadsPath: this.options.downloadsDir,
          headless,
          args: launchArgs,
          ignoreDefaultArgs:
            launchArgs.length > 0 ? ["--disable-extensions"] : undefined,
        },
      );
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
    if (this.pageRegistry.hasObservedContext(context)) {
      return;
    }
    this.pageRegistry.markContextObserved(context);

    context.on("close", () => {
      if (this.context === context) {
        this.context = undefined;
        this.managedPage = undefined;
        this.targetPageId = null;
        this.targetPageCreatedByAdapter = false;
        this.callbacks.clearActiveCapabilityState();
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
      if (!this.options.autoOpenTargetTab) {
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
    forceReload = false,
  ): Promise<void> {
    if (
      !forceReload &&
      this.cdpAttached &&
      matchesTargetLocation(page.url(), targetUrl)
    ) {
      await page.waitForLoadState("domcontentloaded", {
        timeout: NAVIGATION_TIMEOUT_MS,
      });
      return;
    }

    if (forceReload || page.url() !== targetUrl) {
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
            !page.isClosed() &&
            this.pageMatchesTargetWorkspace(page, targetUrl),
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
    const state = await this.callbacks.refreshCapabilities(page);
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
      this.callbacks.clearActiveCapabilityState();
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
}
