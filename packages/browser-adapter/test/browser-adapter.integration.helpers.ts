import { mkdir, mkdtemp } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawn, type ChildProcess } from "node:child_process";
import { chromium } from "playwright-core";
import {
  BrowserAdapter,
  resolveBrowserLaunchConfig,
} from "../src/browserAdapter";
import {
  allocatePort,
  startFakeChatGptServer,
  stopFakeChatGptServer,
} from "./helpers";

export type HarnessUrlOptions = {
  variant?: "classic" | "aria" | "live" | "menu" | "composer-right";
  scenario?:
    | "success"
    | "timeout"
    | "reload"
    | "upload-processing-delay"
    | "upload-processing-stuck"
    | "submit-never-ready"
    | "submit-upload-trap"
    | "submit-upload-trap-no-keyboard"
    | "submit-upload-stale-recover"
    | "followup-rich-text"
    | "thinking-artifact-messages"
    | "followup-stale"
    | "done-early"
    | "stall-recover"
    | "stall-recover-transient"
    | "sticky-stop-after-complete"
    | "sticky-stop-after-reload"
    | "recover-complete"
    | "recover-message-successor"
    | "recover-done-transient"
    | "recover-rich-markdown"
    | "recover-running-transient"
    | "recover-same-message-rewrite"
    | "recover-mismatch"
    | "recover-wrong-conversation"
    | "provider-generation-failure-recover"
    | "provider-generation-failure-after-success"
    | "provider-generation-failure-twice"
    | "research-success"
    | "research-starting-shell"
    | "research-failure"
    | "research-deep-landing"
    | "research-deep-landing-submit-fail"
    | "research-reload";
  missing?: string[];
  conversationStyle?: "query" | "path";
  initialModel?: "instant" | "thinking";
  initialEffort?: "standard" | "extended";
  searchClicksBeforeActivate?: number;
  searchResetsModelOnActivate?: boolean;
  effortInConfigureModal?: boolean;
};

export type BrowserExecutionMode = "managed" | "attach";

export function withHarnessUrl(
  baseUrl: string,
  options: HarnessUrlOptions = {},
): string {
  const url = new URL(baseUrl);
  if (options.variant && options.variant !== "classic") {
    url.searchParams.set("variant", options.variant);
  }
  if (options.scenario && options.scenario !== "success") {
    url.searchParams.set("scenario", options.scenario);
  }
  if (options.missing && options.missing.length > 0) {
    url.searchParams.set("missing", options.missing.join(","));
  }
  if (options.conversationStyle === "query") {
    url.searchParams.set("conversation_style", "query");
  }
  if (options.initialModel) {
    url.searchParams.set("initial_model", options.initialModel);
  }
  if (options.initialEffort) {
    url.searchParams.set("initial_effort", options.initialEffort);
  }
  if (options.searchClicksBeforeActivate) {
    url.searchParams.set(
      "search_clicks_before_activate",
      String(options.searchClicksBeforeActivate),
    );
  }
  if (options.searchResetsModelOnActivate) {
    url.searchParams.set("search_resets_model_on_activate", "true");
  }
  if (options.effortInConfigureModal) {
    url.searchParams.set("effort_in_configure_modal", "true");
  }
  return url.toString();
}

export async function createIsolatedTempDir(
  prefix = "browser",
): Promise<string> {
  const slug = currentTestSlug();
  return mkdtemp(path.join(os.tmpdir(), `tablinq-${prefix}-${slug}-`));
}

export function createBrowserIntegrationHarness() {
  const trackedAdapters: BrowserAdapter[] = [];
  const trackedProcesses: ChildProcess[] = [];

  return {
    trackAdapter(adapter: BrowserAdapter): BrowserAdapter {
      trackedAdapters.push(adapter);
      return adapter;
    },

    async startServer() {
      const server = await startFakeChatGptServer();
      trackedProcesses.push(server.process);
      return server;
    },

    async createWorkspace(prefix = "browser"): Promise<string> {
      return createIsolatedTempDir(prefix);
    },

    async startAttachableBrowser(initialUrl: string) {
      const root = await createIsolatedTempDir("attach-browser");
      const debugPort = await allocatePort();
      const launch = resolveBrowserLaunchConfig(process.platform);
      if (!launch.executablePath) {
        throw new Error(
          "no supported browser executable found for attach tests",
        );
      }

      const userDataDir = path.join(root, "profile");
      await mkdir(userDataDir, { recursive: true });
      const child = spawn(
        launch.executablePath,
        [
          `--remote-debugging-port=${debugPort}`,
          `--user-data-dir=${userDataDir}`,
          "--no-first-run",
          "--no-default-browser-check",
          "--disable-background-networking",
          "--disable-sync",
          "--disable-gpu",
          "--headless=new",
          initialUrl,
        ],
        { stdio: "ignore" },
      );

      trackedProcesses.push(child);
      const endpoint = `http://127.0.0.1:${debugPort}`;
      await waitForDebugEndpoint(endpoint);
      return { process: child, endpoint, browserPath: launch.executablePath };
    },

    async createAdapterForMode(
      mode: BrowserExecutionMode,
      targetUrl: string,
    ): Promise<BrowserAdapter> {
      const root = await createIsolatedTempDir(
        mode === "attach" ? "attach" : "browser",
      );
      if (mode === "attach") {
        const attached = await this.startAttachableBrowser(targetUrl);
        return this.trackAdapter(
          new BrowserAdapter({
            profileDir: path.join(root, "profile"),
            downloadsDir: path.join(root, "downloads"),
            defaultTargetUrl: targetUrl,
            connectionMode: "attach_first",
            debugEndpoint: attached.endpoint,
          }),
        );
      }

      return this.trackAdapter(
        new BrowserAdapter({
          profileDir: path.join(root, "profile"),
          downloadsDir: path.join(root, "downloads"),
          defaultTargetUrl: targetUrl,
        }),
      );
    },

    async cleanup(): Promise<void> {
      await Promise.allSettled(
        trackedAdapters
          .slice()
          .reverse()
          .map((adapter) => adapter.shutdown()),
      );
      await Promise.allSettled(
        trackedProcesses
          .slice()
          .reverse()
          .map((process) => stopFakeChatGptServer(process)),
      );
    },
  };
}

async function waitForDebugEndpoint(endpoint: string): Promise<void> {
  const deadline = Date.now() + 10000;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(`${endpoint}/json/version`);
      if (response.ok) {
        return;
      }
    } catch {}
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`debug endpoint did not become ready at ${endpoint}`);
}

export async function listAttachedPages(endpoint: string): Promise<string[]> {
  const browser = await chromium.connectOverCDP(endpoint);
  try {
    return browser
      .contexts()
      .flatMap((context) => context.pages())
      .map((page) => page.url());
  } finally {
    await browser.close();
  }
}

export async function withAttachedBrowser<T>(
  endpoint: string,
  callback: (
    browser: Awaited<ReturnType<typeof chromium.connectOverCDP>>,
  ) => Promise<T>,
): Promise<T> {
  const browser = await chromium.connectOverCDP(endpoint);
  try {
    return await callback(browser);
  } finally {
    await browser.close();
  }
}

function currentTestSlug(): string {
  const testName =
    (
      globalThis as {
        expect?: { getState?: () => { currentTestName?: string } };
      }
    ).expect?.getState?.().currentTestName ?? "test";
  return (
    String(testName)
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "")
      .slice(0, 48) || "test"
  );
}
