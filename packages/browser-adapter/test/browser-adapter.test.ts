import { mkdir, mkdtemp, readFile, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawn, type ChildProcess } from "node:child_process";
import { JSDOM } from "jsdom";
import { chromium } from "playwright-core";
import {
  BrowserAdapter,
  chooseUniqueArtifactPath,
  conversationIdFromUrl,
  deepResearchStartDelayMs,
  detectBrowserExecutable,
  interactionJitterMs,
  MAX_UPLOAD_FILE_BYTES,
  resolveConversationUrl,
  resolveBrowserLaunchConfig,
  uploadAckTimeoutMsForFiles,
  workspaceKeyFromUrl,
} from "../src/browserAdapter";
import {
  allocatePort,
  startFakeChatGptServer,
  stopFakeChatGptServer,
} from "./helpers";

function withHarnessUrl(
  baseUrl: string,
  options: {
    variant?: "classic" | "aria" | "live" | "menu";
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
  } = {},
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
  return url.toString();
}

let trackedAdapters: BrowserAdapter[] = [];
let trackedServers: ChildProcess[] = [];
type BrowserExecutionMode = "managed" | "attach";

function trackAdapter(adapter: BrowserAdapter): BrowserAdapter {
  trackedAdapters.push(adapter);
  return adapter;
}

async function startTrackedFakeChatGptServer() {
  const server = await startFakeChatGptServer();
  trackedServers.push(server.process);
  return server;
}

async function createTrackedAdapterForMode(
  mode: BrowserExecutionMode,
  targetUrl: string,
): Promise<BrowserAdapter> {
  const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
  if (mode === "attach") {
    const attached = await startTrackedAttachableBrowser(targetUrl);
    return trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );
  }

  return trackAdapter(
    new BrowserAdapter({
      profileDir: path.join(root, "profile"),
      downloadsDir: path.join(root, "downloads"),
      defaultTargetUrl: targetUrl,
    }),
  );
}

async function startTrackedAttachableBrowser(initialUrl: string) {
  const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-attach-browser-"));
  const debugPort = await allocatePort();
  const launch = resolveBrowserLaunchConfig(process.platform);
  if (!launch.executablePath) {
    throw new Error("no supported browser executable found for attach tests");
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
    {
      stdio: "ignore",
    },
  );

  trackedServers.push(child);
  const endpoint = `http://127.0.0.1:${debugPort}`;
  await waitForDebugEndpoint(endpoint);
  return { process: child, endpoint, browserPath: launch.executablePath };
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

async function listAttachedPages(endpoint: string): Promise<string[]> {
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

async function withAttachedBrowser<T>(
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

beforeEach(() => {
  trackedAdapters = [];
  trackedServers = [];
});

afterEach(async () => {
  await Promise.allSettled(
    trackedAdapters
      .slice()
      .reverse()
      .map((adapter) => adapter.shutdown()),
  );
  await Promise.allSettled(
    trackedServers
      .slice()
      .reverse()
      .map((server) => stopFakeChatGptServer(server)),
  );
});

describe("browser adapter", () => {
  it("launches a managed browser and opens the fake harness", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
      }),
    );

    const snapshot = await adapter.open({
      targetUrl: server.url,
      headless: true,
    });

    expect(snapshot.running).toBe(true);
    expect(snapshot.current_url).toContain(server.url);
    expect(snapshot.authenticated).toBe(true);
    expect(snapshot.extension_connected).toBe(true);
    expect(snapshot.capability_report?.matrix.composer_ready).toBe(true);
    expect(snapshot.attach_mode).toBe("managed_only");
    expect(snapshot.browser_owner).toBe("managed");
    expect(snapshot.cdp_attached).toBe(false);
    expect(snapshot.target_tab_found).toBe(true);
    expect(snapshot.target_tab_id).toBeTruthy();
    expect(snapshot.target_workspace_key).toBe(new URL(server.url).toString());
    expect(snapshot.debug_endpoint).toBe("http://127.0.0.1:9222");
    expect(snapshot.auth_source).toBe("page");
    expect(snapshot.extension_required).toBe(true);
    expect(snapshot.last_capability_report_at).toBeTruthy();

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it.each([
    ["classic", "classic"],
    ["aria", "aria"],
    ["menu", "menu"],
    ["live", "live"],
  ] as const)(
    "captures the harness capability matrix for the %s variant",
    async (_label, variant) => {
      const server = await startTrackedFakeChatGptServer();
      const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
      const targetUrl = withHarnessUrl(server.url, {
        variant,
        initialModel: "thinking",
      });
      const adapter = trackAdapter(
        new BrowserAdapter({
          profileDir: path.join(root, "profile"),
          downloadsDir: path.join(root, "downloads"),
          defaultTargetUrl: targetUrl,
        }),
      );

      const snapshot = await adapter.open({
        targetUrl,
        headless: true,
      });

      expect(snapshot.extension_connected).toBe(true);
      expect(snapshot.capability_report?.matrix).toEqual({
        composer_ready: true,
        submit_available: true,
        model_picker_visible: true,
        thinking_effort_visible: true,
        search_mode_visible: true,
        research_mode_visible: true,
        upload_control_visible: true,
        downloadable_artifacts_observable: true,
      });
      expect(
        snapshot.capability_report?.aliases.model_classes.instant,
      ).toBeTruthy();
      expect(
        snapshot.capability_report?.aliases.model_classes.thinking,
      ).toBeTruthy();
      expect(snapshot.capability_report?.aliases.efforts.standard).toBeTruthy();
      expect(snapshot.capability_report?.aliases.efforts.extended).toBeTruthy();
      const diagnostics = await adapter.diagnostics();
      expect(diagnostics.selector_pack?.pack_id).toBe("chatgpt-default");
      expect(diagnostics.strategy_attempts.length).toBeGreaterThan(3);
      expect(diagnostics.strategy_telemetry.length).toBeGreaterThan(0);
      expect(adapter.getLastDomNeighborhoodsForTest().length).toBeGreaterThan(
        3,
      );

      await adapter.shutdown();
      await stopFakeChatGptServer(server.process);
    },
  );

  it("keeps the effort control absent until Thinking is selected on the modern menu harness", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      variant: "menu",
      initialModel: "instant",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );

    const snapshot = await adapter.open({ targetUrl, headless: true });
    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      modelTriggerLabel:
        document
          .querySelector("#model-switcher-trigger")
          ?.textContent?.replace(/\s+/g, " ")
          .trim() ?? null,
      modelTriggerAria:
        document
          .querySelector("#model-switcher-trigger")
          ?.getAttribute("aria-label") ?? null,
      effortVisible: !(
        document
          .querySelector("#thinking-effort-trigger")
          ?.hasAttribute("hidden") ?? true
      ),
      menuItems: [
        document
          .querySelector("#model-option-instant")
          ?.textContent?.replace(/\s+/g, " ")
          .trim() ?? "",
        document
          .querySelector("#model-option-thinking")
          ?.textContent?.replace(/\s+/g, " ")
          .trim() ?? "",
        document
          .querySelector("#model-option-pro")
          ?.textContent?.replace(/\s+/g, " ")
          .trim() ?? "",
      ],
    }));

    expect(snapshot.capability_report?.matrix.thinking_effort_visible).toBe(
      false,
    );
    expect(state).toEqual({
      modelTriggerLabel: "ChatGPT",
      modelTriggerAria: "Model selector",
      effortVisible: false,
      menuItems: [
        "Instant For everyday chats",
        "Thinking For complex questions",
        "Pro Research-grade intelligence",
      ],
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("captures only Deep research aliases from the live sidebar nav cluster", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, { variant: "live" });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );

    const snapshot = await adapter.open({ targetUrl, headless: true });
    const researchAliases =
      snapshot.capability_report?.aliases.modes.research ?? [];

    expect(snapshot.capability_report?.matrix.research_mode_visible).toBe(true);
    expect(researchAliases).toContain("deep research");
    expect(researchAliases).not.toContain("apps");
    expect(researchAliases).not.toContain("codex");
    expect(researchAliases).not.toContain("gpts");
    expect(researchAliases).not.toContain("projects");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("bounds the Deep Research start delay jitter to the real user range", () => {
    expect(deepResearchStartDelayMs(0)).toBe(4_000);
    expect(deepResearchStartDelayMs(0.5)).toBe(7_000);
    expect(deepResearchStartDelayMs(1)).toBe(10_000);
  });

  it("bounds short interaction jitters to human-scale ranges", () => {
    expect(interactionJitterMs("chip_clear", 0)).toBe(30);
    expect(interactionJitterMs("chip_clear", 1)).toBe(80);
    expect(interactionJitterMs("keyboard_submit", 0)).toBe(35);
    expect(interactionJitterMs("keyboard_submit", 1)).toBe(80);
    expect(interactionJitterMs("picker_trigger", 0)).toBe(45);
    expect(interactionJitterMs("picker_trigger", 1)).toBe(120);
    expect(interactionJitterMs("choice_select", 0)).toBe(55);
    expect(interactionJitterMs("choice_select", 1)).toBe(145);
    expect(interactionJitterMs("mode_toggle", 0)).toBe(60);
    expect(interactionJitterMs("mode_toggle", 1)).toBe(150);
    expect(interactionJitterMs("upload_entry", 0)).toBe(70);
    expect(interactionJitterMs("upload_entry", 1)).toBe(160);
    expect(interactionJitterMs("button_click", 0)).toBe(70);
    expect(interactionJitterMs("button_click", 1)).toBe(170);
  });

  it("scales upload acknowledgment timeout nonlinearly by file size and additional files", () => {
    expect(uploadAckTimeoutMsForFiles([])).toBe(10_000);
    expect(uploadAckTimeoutMsForFiles([0])).toBe(10_000);
    expect(uploadAckTimeoutMsForFiles([MAX_UPLOAD_FILE_BYTES])).toBe(40_000);
    expect(uploadAckTimeoutMsForFiles([MAX_UPLOAD_FILE_BYTES / 4])).toBe(
      25_000,
    );
    expect(
      uploadAckTimeoutMsForFiles([
        MAX_UPLOAD_FILE_BYTES,
        MAX_UPLOAD_FILE_BYTES / 4,
      ]),
    ).toBe(62_500);
  });

  it("reuses the managed page on repeated open", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
      }),
    );

    await adapter.open({ targetUrl: server.url, headless: true });
    const firstPage = adapter.getManagedPageForTest();
    await adapter.open({ targetUrl: server.url, headless: true });
    const secondPage = adapter.getManagedPageForTest();

    expect(firstPage).toBeDefined();
    expect(secondPage).toBeDefined();
    expect(firstPage).toBe(secondPage);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("reuses the persistent profile across adapter restarts", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const profileDir = path.join(root, "profile");
    const downloadsDir = path.join(root, "downloads");

    const first = trackAdapter(
      new BrowserAdapter({
        profileDir,
        downloadsDir,
        defaultTargetUrl: server.url,
      }),
    );
    await first.open({ targetUrl: server.url, headless: true });
    await first.shutdown();

    const second = trackAdapter(
      new BrowserAdapter({
        profileDir,
        downloadsDir,
        defaultTargetUrl: server.url,
      }),
    );
    const snapshot = await second.open({
      targetUrl: server.url,
      headless: true,
    });

    expect(snapshot.running).toBe(true);
    expect(snapshot.profile_dir).toBe(profileDir);

    await second.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("reconnects after the managed tab is closed", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
      }),
    );

    await adapter.open({ targetUrl: server.url, headless: true });
    await adapter.getManagedPageForTest()!.close();

    const snapshot = await adapter.health();
    expect(snapshot.running).toBe(true);
    expect(snapshot.current_url).toContain(server.url);
    expect(snapshot.extension_connected).toBe(true);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("captures screenshots on demand", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
      }),
    );
    await adapter.open({ targetUrl: server.url, headless: true });

    const screenshotPath = path.join(root, "artifacts", "managed-page.png");
    const snapshot = await adapter.screenshot(screenshotPath);
    const file = await stat(screenshotPath);

    expect(file.size).toBeGreaterThan(0);
    expect(snapshot.screenshot_path).toBe(screenshotPath);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("detects reloads on the managed page", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
      }),
    );
    await adapter.open({ targetUrl: server.url, headless: true });

    await adapter
      .getManagedPageForTest()!
      .reload({ waitUntil: "domcontentloaded" });
    const snapshot = await adapter.health();

    expect(snapshot.reload_count).toBeGreaterThan(0);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("attaches to an existing browser session", async () => {
    const server = await startTrackedFakeChatGptServer();
    const attached = await startTrackedAttachableBrowser(server.url);
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );

    const snapshot = await adapter.open({
      targetUrl: server.url,
      headless: true,
    });

    expect(snapshot.running).toBe(true);
    expect(snapshot.browser_owner).toBe("external");
    expect(snapshot.cdp_attached).toBe(true);
    expect(snapshot.current_url).toContain(server.url);
    expect(snapshot.target_tab_found).toBe(true);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("reuses an existing ChatGPT tab in attach mode", async () => {
    const server = await startTrackedFakeChatGptServer();
    const attached = await startTrackedAttachableBrowser(server.url);
    const before = await listAttachedPages(attached.endpoint);
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );

    await adapter.open({ targetUrl: server.url, headless: true });
    const after = await listAttachedPages(attached.endpoint);

    expect(after.length).toBe(before.length);
    expect(after.some((url) => url.includes(server.url))).toBe(true);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("opens a new target tab when none exists in attach mode", async () => {
    const server = await startTrackedFakeChatGptServer();
    const attached = await startTrackedAttachableBrowser("about:blank");
    const before = await listAttachedPages(attached.endpoint);
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );

    const snapshot = await adapter.open({
      targetUrl: server.url,
      headless: true,
    });
    const after = await listAttachedPages(attached.endpoint);

    expect(snapshot.browser_owner).toBe("external");
    expect(snapshot.current_url).toContain(server.url);
    expect(before.every((url) => !url.includes(server.url))).toBe(true);
    expect(after.some((url) => url.includes(server.url))).toBe(true);

    await adapter.close();
    const closed = await listAttachedPages(attached.endpoint);
    expect(closed.length).toBeGreaterThan(0);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers after the attached target tab is closed", async () => {
    const server = await startTrackedFakeChatGptServer();
    const attached = await startTrackedAttachableBrowser(server.url);
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );

    await adapter.open({ targetUrl: server.url, headless: true });
    await adapter.getManagedPageForTest()!.close();

    const snapshot = await adapter.health();
    expect(snapshot.running).toBe(true);
    expect(snapshot.browser_owner).toBe("external");
    expect(snapshot.current_url).toContain(server.url);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("reopens the attached ChatGPT tab and completes ask after the target tab is closed", async () => {
    const server = await startTrackedFakeChatGptServer();
    const attached = await startTrackedAttachableBrowser(server.url);
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );

    await adapter.open({ targetUrl: server.url, headless: true });
    await adapter.getManagedPageForTest()!.close();

    const result = await adapter.ask(
      {
        jobId: "job-ask-reopen-closed-attached-tab",
        prompt: "reopen the attached tab and answer",
      },
      () => {},
    );
    const workspacePages = (await listAttachedPages(attached.endpoint)).filter(
      (url) => workspaceKeyFromUrl(url) === workspaceKeyFromUrl(server.url),
    );

    expect(result.output_text).toBe(
      "Fake response for: reopen the attached tab and answer",
    );
    expect(workspacePages).toHaveLength(1);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("counts reloads in attach mode", async () => {
    const server = await startTrackedFakeChatGptServer();
    const attached = await startTrackedAttachableBrowser(server.url);
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );

    await adapter.open({ targetUrl: server.url, headless: true });
    await adapter
      .getManagedPageForTest()!
      .reload({ waitUntil: "domcontentloaded" });

    const snapshot = await adapter.health();
    expect(snapshot.reload_count).toBeGreaterThan(0);
    expect(snapshot.browser_owner).toBe("external");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("captures diagnostics screenshots in attach mode", async () => {
    const server = await startTrackedFakeChatGptServer();
    const attached = await startTrackedAttachableBrowser(server.url);
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );
    await adapter.open({ targetUrl: server.url, headless: true });

    const diagnostics = await adapter.diagnostics(path.join(root, "bundle"));
    const screenshot = await stat(diagnostics.screenshot_path!);

    expect(diagnostics.screenshot_path).toBeTruthy();
    expect(screenshot.size).toBeGreaterThan(0);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("keeps active tab metadata for attach mode", async () => {
    const server = await startTrackedFakeChatGptServer();
    const attached = await startTrackedAttachableBrowser(server.url);
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );

    const snapshot = await adapter.open({
      targetUrl: server.url,
      headless: true,
    });

    expect(snapshot.target_tab_id).toBeTruthy();
    expect(snapshot.target_workspace_key).toBe(new URL(server.url).toString());
    expect(snapshot.last_capability_report_at).toBeTruthy();

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("waits briefly for extension readiness after attach", async () => {
    const server = await startTrackedFakeChatGptServer();
    const targetUrl = withHarnessUrl(server.url, { variant: "menu" });
    const attached = await startTrackedAttachableBrowser(targetUrl);
    await withAttachedBrowser(attached.endpoint, async (browser) => {
      const page = browser.contexts()[0]!.pages()[0]!;
      await page.evaluate(() => {
        const controls = Array.from(
          document.querySelectorAll(
            "#prompt-textarea, [data-testid='send-button'], #model-switcher-trigger, #thinking-effort-trigger, [data-testid='upload-button']",
          ),
        ) as HTMLElement[];
        for (const control of controls) {
          control.dataset.originalDisplay = control.style.display;
          control.style.display = "none";
        }
        setTimeout(() => {
          for (const control of controls) {
            control.style.display = control.dataset.originalDisplay ?? "";
          }
        }, 600);
      });
    });

    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );

    const snapshot = await adapter.open({
      targetUrl,
      headless: true,
    });

    expect(snapshot.extension_connected).toBe(true);
    expect(snapshot.capability_report?.matrix.composer_ready).toBe(true);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("switches to the matching attached tab that has a working extension report", async () => {
    const server = await startTrackedFakeChatGptServer();
    const attached = await startTrackedAttachableBrowser(server.url);
    const preferredUrl = new URL("c/preferred", server.url).toString();
    await withAttachedBrowser(attached.endpoint, async (browser) => {
      const context = browser.contexts()[0]!;
      const first = context.pages()[0]!;
      await first.evaluate(() => {
        document.body.innerHTML = "<main>stale tab</main>";
      });

      const second = await context.newPage();
      await second.goto(preferredUrl, { waitUntil: "domcontentloaded" });
    });

    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );

    await adapter.open({
      targetUrl: server.url,
      headless: true,
    });
    const selected = await adapter.getManagedPageForTest()!.url();

    expect(selected).toContain("/c/preferred");
    expect((await adapter.health()).extension_connected).toBe(true);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("closes stale attached ChatGPT tabs after selecting a healthy target tab", async () => {
    const server = await startTrackedFakeChatGptServer();
    const attached = await startTrackedAttachableBrowser(server.url);
    await withAttachedBrowser(attached.endpoint, async (browser) => {
      const context = browser.contexts()[0]!;
      const stale = context.pages()[0]!;
      await stale.evaluate(() => {
        document.body.innerHTML = "<main>stale tab</main>";
      });

      const healthy = await context.newPage();
      await healthy.goto(server.url, { waitUntil: "domcontentloaded" });
    });

    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );

    await adapter.open({
      targetUrl: server.url,
      headless: true,
    });
    const workspacePages = (await listAttachedPages(attached.endpoint)).filter(
      (url) => workspaceKeyFromUrl(url) === workspaceKeyFromUrl(server.url),
    );

    expect(workspacePages).toHaveLength(1);
    expect(await adapter.getManagedPageForTest()!.url()).toContain(server.url);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("uses /c/<id> as the only conversation identity form", () => {
    const current = "https://chatgpt.com/c/conv-9?variant=live";
    const workspace = "https://chatgpt.com/?variant=live";

    expect(conversationIdFromUrl(workspace)).toBeNull();
    expect(conversationIdFromUrl(current)).toBe("conv-9");
    expect(workspaceKeyFromUrl(current)).toBe(workspace);
    expect(resolveConversationUrl(workspace, "conv-9")).toBe(current);
  });

  it("falls back to managed launch when attach mode is configured as managed_fallback", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "managed_fallback",
        debugEndpoint: "http://127.0.0.1:9",
      }),
    );

    const snapshot = await adapter.open({
      targetUrl: server.url,
      headless: true,
    });

    expect(snapshot.running).toBe(true);
    expect(snapshot.browser_owner).toBe("managed");
    expect(snapshot.cdp_attached).toBe(false);
    expect(snapshot.current_url).toContain(server.url);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("streams ask responses to completion", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
      }),
    );
    await adapter.open({ targetUrl: server.url, headless: true });

    const chunks: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-success",
        prompt: "hello streaming world",
      },
      (chunk: any) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );

    expect(chunks.join("")).toBe("Fake response for: hello streaming world");
    expect(result.output_text).toBe("Fake response for: hello streaming world");
    expect(result.conversation_id).toMatch(/^conv-/);
    expect(result.message_id).toMatch(/^msg-/);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("does not replay the previous reply when a follow-up ask reuses the latest conversation implicitly", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      scenario: "followup-stale",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const first = await adapter.ask(
      {
        jobId: "job-ask-followup-implicit-first",
        prompt: "hello gpt",
      },
      () => {},
    );

    const secondChunks: string[] = [];
    const second = await adapter.ask(
      {
        jobId: "job-ask-followup-implicit-second",
        prompt: "what can you do?",
      },
      (chunk) => {
        if (chunk.kind === "delta") {
          secondChunks.push(chunk.text);
        }
      },
    );

    expect(second.conversation_id).toBe(first.conversation_id);
    expect(second.output_text).toBe("Fake response for: what can you do?");
    expect(secondChunks.join("")).toBe(second.output_text);
    expect(secondChunks.join("")).not.toContain(first.output_text);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("does not replay the previous reply when a follow-up ask uses the explicit conversation id", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      scenario: "followup-stale",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const first = await adapter.ask(
      {
        jobId: "job-ask-followup-explicit-first",
        prompt: "hello gpt",
      },
      () => {},
    );

    const secondChunks: string[] = [];
    const second = await adapter.ask(
      {
        jobId: "job-ask-followup-explicit-second",
        prompt: "what can you do?",
        conversationId: first.conversation_id,
      },
      (chunk) => {
        if (chunk.kind === "delta") {
          secondChunks.push(chunk.text);
        }
      },
    );

    expect(second.conversation_id).toBe(first.conversation_id);
    expect(second.output_text).toBe("Fake response for: what can you do?");
    expect(secondChunks.join("")).toBe(second.output_text);
    expect(secondChunks.join("")).not.toContain(first.output_text);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("streams a multi-paragraph follow-up ask without repeating earlier text or failing reconciliation", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "followup-rich-text",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const first = await adapter.ask(
      {
        jobId: "job-ask-followup-rich-text-first",
        prompt: "hey",
      },
      () => {},
    );

    const secondChunks: string[] = [];
    const second = await adapter.ask(
      {
        jobId: "job-ask-followup-rich-text-second",
        prompt: "what are your capabilities?",
      },
      (chunk) => {
        if (chunk.kind === "delta") {
          secondChunks.push(chunk.text);
        }
      },
    );

    const expected =
      "I can help with thinking work and doing work.\n\n" +
      "I can explain concepts, write and edit, summarize, brainstorm, tutor, code, debug, analyze data, compare options, plan projects, draft emails/messages, and turn rough notes into polished documents.";

    expect(second.conversation_id).toBe(first.conversation_id);
    expect(second.artifacts).toHaveLength(0);
    expect(second.output_text).toBe(expected);
    expect(secondChunks.join("")).toBe(expected);
    expect(secondChunks.join("")).not.toContain(
      `${first.output_text}\n${first.output_text}`,
    );

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("suppresses thought artifact messages and streams the actual reply", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "thinking-artifact-messages",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const chunks: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-thinking-artifact-messages",
        prompt: "reason carefully about this tradeoff",
      },
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );

    expect(result.output_text).toBe(
      "Fake response for: reason carefully about this tradeoff",
    );
    expect(chunks.join("")).toBe(result.output_text);
    expect(chunks.length).toBeGreaterThan(1);
    expect(chunks.join("")).not.toContain("I’m reconstructing the prior context");
    expect(chunks.join("")).not.toContain("The comparison is narrowing");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("serializes rich assistant markdown into terminal-safe markdown text", async () => {
    const runtimeSource = await readFile(
      path.join(
        process.cwd(),
        "packages/browser-adapter/src/pageAdapter.runtime.js",
      ),
      "utf8",
    );
    const expected = [
      "# Heading One",
      "## Heading Two",
      "",
      "Plain text line.",
      "",
      "1. First numbered item",
      "2. Second numbered item",
      "",
      "- Bullet one",
      "- Bullet two",
      "",
      "> Blockquote line",
      "",
      "Inline `code` example.",
      "",
      "```python",
      "def hello():",
      '    return "world"',
      "```",
      "",
      "**bold**",
      "*italic*",
      "***bold italic***",
      "",
      "| col1 | col2 |",
      "| --- | --- |",
      "| A | B |",
      "",
      "Final sentence.",
    ].join("\n");

    const dom = new JSDOM("<!doctype html><html><body></body></html>", {
      url: "http://127.0.0.1:3847/?variant=live",
      runScripts: "outside-only",
    });
    const { window } = dom;
    const article = window.document.createElement("article");
    article.setAttribute("data-slot", "assistant-message");
    article.hidden = false;
    article.setAttribute("data-message-author-role", "assistant");
    article.setAttribute("data-message-id", "msg-rich-markdown");
    article.innerHTML = `
          <div class="markdown prose dark:prose-invert w-full wrap-break-word light markdown-new-styling">
            <h1>Heading One</h1>
            <h2>Heading Two</h2>
            <p>Plain text line.</p>
            <ol>
              <li><p>First numbered item</p></li>
              <li><p>Second numbered item</p></li>
            </ol>
            <ul>
              <li><p>Bullet one</p></li>
              <li><p>Bullet two</p></li>
            </ul>
            <blockquote><p>Blockquote line</p></blockquote>
            <p>Inline <code>code</code> example.</p>
            <pre>
              <div class="text-sm font-medium">Python</div>
              <button aria-label="Copy"></button>
              <button aria-label="Run code">Run</button>
              <div class="cm-content">def hello():\n    return "world"</div>
            </pre>
            <p><strong>bold</strong><br><em>italic</em><br><em><strong>bold italic</strong></em></p>
            <div class="table-container">
              <table>
                <thead>
                  <tr><th>col1</th><th>col2</th></tr>
                </thead>
                <tbody>
                  <tr><td>A</td><td>B</td></tr>
                </tbody>
              </table>
              <button aria-label="Copy table"></button>
            </div>
            <p>Final sentence.</p>
          </div>
        `;
    window.document.body.append(article);
    window.document.body.dataset.conversationId = "conv-rich-markdown";
    window.document.body.dataset.lastMessageId = "msg-rich-markdown";
    window.document.body.dataset.streamState = "done";

    const snapshot = window.eval(
      `${runtimeSource.replace(
        /^export\s+/gm,
        "",
      )}\ncreateTablinqPageAdapterRuntime().readAskSnapshot();`,
    );

    expect(snapshot.text).toBe(expected);
    dom.window.close();
  });

  it("refuses asks when the managed page is a signed-out local login screen", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: `${server.url}/login-expired.html`,
      }),
    );
    await adapter.open({
      targetUrl: `${server.url}/login-expired.html`,
      headless: true,
    });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-auth-expired",
          prompt: "should refuse while signed out",
        },
        () => {},
      ),
    ).rejects.toThrow("auth required");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("selects Instant from the current ChatGPT model menu", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      variant: "menu",
      initialModel: "thinking",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    await adapter.ask(
      {
        jobId: "job-ask-menu-instant",
        prompt: "configure current menu instant",
        mode: "search",
        modelClass: "instant",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      model: document.body.dataset.selectedModel,
    }));

    expect(state).toEqual({
      mode: "search",
      model: "instant",
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("selects Instant from the classic inline model controls", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      initialModel: "thinking",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    await adapter.ask(
      {
        jobId: "job-ask-classic-instant",
        prompt: "configure classic instant",
        modelClass: "instant",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
    }));

    expect(state).toEqual({
      model: "instant",
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("selects Thinking and Extended from the aria inline controls", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      variant: "aria",
      initialModel: "instant",
      initialEffort: "standard",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    await adapter.ask(
      {
        jobId: "job-ask-aria-thinking-extended",
        prompt: "configure aria thinking",
        modelClass: "thinking",
        effort: "extended",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
    }));

    expect(state).toEqual({
      model: "thinking",
      effort: "extended",
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("starts a fresh conversation when a new ask requests one", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const first = await adapter.ask(
      {
        jobId: "job-ask-first-conversation",
        prompt: "start the first conversation",
      },
      () => {},
    );
    const fresh = await adapter.ask(
      {
        jobId: "job-ask-fresh-conversation",
        prompt: "start a fresh conversation",
        newConversation: true,
      },
      () => {},
    );

    expect(fresh.conversation_id).not.toBe(first.conversation_id);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("selects Thinking from the current ChatGPT model menu and then applies Extended effort", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      variant: "menu",
      initialModel: "instant",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    await adapter.ask(
      {
        jobId: "job-ask-menu-thinking-extended",
        prompt: "configure current menu thinking",
        mode: "search",
        modelClass: "thinking",
        effort: "extended",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
    }));

    expect(state).toEqual({
      mode: "search",
      model: "thinking",
      effort: "extended",
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("reports capability missing when the requested model option is absent from the picker", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      variant: "menu",
      initialModel: "thinking",
      missing: ["instant-model"],
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-menu-missing-instant",
          prompt: "configure missing instant",
          modelClass: "instant",
        },
        () => {},
      ),
    ).rejects.toThrow(
      /ui drift: could not find a concrete control for model=instant/i,
    );

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("reports capability missing when thinking effort controls never appear", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      variant: "menu",
      initialModel: "instant",
      missing: ["effort"],
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-menu-missing-effort",
          prompt: "configure missing effort",
          modelClass: "thinking",
          effort: "extended",
        },
        () => {},
      ),
    ).rejects.toThrow(
      /capability missing: thinking effort; requested=extended/i,
    );

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("runs ask --model instant on the current live harness", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
          initialModel: "thinking",
          initialEffort: "extended",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        initialModel: "thinking",
        initialEffort: "extended",
      }),
      headless: true,
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-live-instant",
        prompt: "configure live instant",
        modelClass: "instant",
        files: [fixture],
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      effortVisible: !(
        document
          .querySelector("#thinking-effort-trigger")
          ?.hasAttribute("hidden") ?? true
      ),
    }));

    expect(result.output_text).toBe(
      "Fake response for: configure live instant",
    );
    expect(result.artifacts).toHaveLength(1);
    expect(state).toEqual({
      model: "instant",
      effortVisible: false,
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("drives the live-like harness through ask, download recovery, and ready-state checks", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, { variant: "live" }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { variant: "live" }),
      headless: true,
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-live-variant",
        prompt: "configure live",
        mode: "research",
        modelClass: "thinking",
        effort: "extended",
        files: [fixture],
      },
      () => {},
    );
    const artifactBody = await readFile(
      result.artifacts[0]!.storage_path,
      "utf8",
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      currentUrl: window.location.href,
      composerReady: Boolean(document.querySelector("#prompt-textarea")),
      researchLandingVisited: document.body.dataset.researchLandingVisited,
      sendHidden:
        document
          .querySelector("[data-testid='send-button']")
          ?.hasAttribute("hidden") ?? false,
      stopHidden:
        document
          .querySelector("[data-testid='stop-button']")
          ?.hasAttribute("hidden") ?? true,
    }));

    expect(state).toEqual({
      currentUrl: resolveConversationUrl(
        withHarnessUrl(server.url, {
          variant: "live",
        }),
        result.conversation_id,
      ),
      composerReady: true,
      researchLandingVisited: "true",
      sendHidden: false,
      stopHidden: true,
    });
    expect(result.output_text).toContain("Research report for: configure live");
    expect(result.artifacts).toHaveLength(1);
    expect(artifactBody).toContain("Mode: research");
    expect(artifactBody).toContain("Model: thinking");
    expect(artifactBody).toContain("Effort: extended");
    expect(artifactBody).toContain("Uploads: brief.txt");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails with a capability-missing error when search mode is unavailable", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, { missing: ["search"] }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { missing: ["search"] }),
      headless: true,
    });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-missing-search",
          prompt: "search for docs",
          mode: "search",
        },
        () => {},
      ),
    ).rejects.toThrow(/capability missing: search mode/i);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("uploads files and persists generated artifacts", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixtureDir = path.join(root, "fixtures");
    const firstFile = path.join(fixtureDir, "brief.txt");
    const secondFile = path.join(fixtureDir, "notes.csv");
    await mkdir(fixtureDir, { recursive: true });
    await writeFile(firstFile, "brief body", "utf8");
    await writeFile(secondFile, "col\nvalue", "utf8");

    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
      }),
    );
    await adapter.open({ targetUrl: server.url, headless: true });

    const result = await adapter.ask(
      {
        jobId: "job-ask-upload-artifact",
        prompt: "summarize uploads",
        files: [firstFile, secondFile],
      },
      () => {},
    );

    expect(result.artifacts).toHaveLength(1);
    expect(result.artifacts[0]!.file_name).toContain("brief-summary");
    expect(result.artifacts[0]!.content_type).toBe("text/plain; charset=utf-8");
    expect(result.artifacts[0]!.size_bytes).toBeGreaterThan(0);
    const downloaded = await stat(result.artifacts[0]!.storage_path);
    expect(downloaded.size).toBeGreaterThan(0);
    const artifactBody = await readFile(
      result.artifacts[0]!.storage_path,
      "utf8",
    );
    expect(artifactBody).toContain("Uploads: brief.txt, notes.csv");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("runs research jobs with progress updates and persisted artifacts", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixtureDir = path.join(root, "fixtures");
    const firstFile = path.join(fixtureDir, "brief.txt");
    const secondFile = path.join(fixtureDir, "notes.csv");
    await mkdir(fixtureDir, { recursive: true });
    await writeFile(firstFile, "brief body", "utf8");
    await writeFile(secondFile, "col\nvalue", "utf8");
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
          scenario: "research-success",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        scenario: "research-success",
      }),
      headless: true,
    });

    const updates: string[] = [];
    const result = await adapter.research(
      {
        jobId: "job-research-success",
        query: "investigate launch risks",
        files: [firstFile, secondFile],
        timeoutMs: 5_000,
      },
      (update) => {
        updates.push(update.status);
      },
    );

    expect(updates).toContain("starting");
    expect(updates).toContain("waiting_for_sources");
    expect(updates).toContain("running");
    expect(result.report_text).toContain(
      "Research report for: investigate launch risks",
    );
    expect(result.artifacts).toHaveLength(1);
    const artifactBody = await readFile(
      result.artifacts[0]!.storage_path,
      "utf8",
    );
    expect(artifactBody).toContain("Uploads: brief.txt, notes.csv");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("does not finalize Deep Research early when a post-start shell shows provisional text", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
          scenario: "research-starting-shell",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        scenario: "research-starting-shell",
      }),
      headless: true,
    });

    const updates: string[] = [];
    const result = await adapter.research(
      {
        jobId: "job-research-starting-shell",
        query:
          "analyze browser-backed provider tradeoffs across latency, privacy, resilience, and deployment constraints",
        timeoutMs: 8_000,
      },
      (update) => {
        updates.push(update.status);
      },
    );

    expect(updates).toContain("starting");
    expect(updates).toContain("waiting_for_sources");
    expect(updates).toContain("running");
    expect(result.report_text).toContain(
      "Research report for: analyze browser-backed provider tradeoffs across latency, privacy, resilience, and deployment constraints",
    );
    expect(result.report_text).not.toContain(
      "Browser vs Hosted AI for Enterprise",
    );
    expect(result.report_text).not.toContain("Starting...");
    expect(result.report_text).not.toContain("Stop research");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("submits research from the /deep-research landing page and completes normally", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
          scenario: "research-deep-landing",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        scenario: "research-deep-landing",
      }),
      headless: true,
    });

    const updates: string[] = [];
    const result = await adapter.research(
      {
        jobId: "job-research-deep-landing",
        query: "investigate deep research landing submission",
        files: [fixture],
        timeoutMs: 8_000,
      },
      (update) => {
        updates.push(update.status);
      },
    );
    const pageState = await adapter.getManagedPageForTest()!.evaluate(() => ({
      researchLandingVisited: document.body.dataset.researchLandingVisited,
      researchSubmitted: document.body.dataset.researchSubmitted,
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
    }));

    expect(result.report_text).toContain(
      "Research report for: investigate deep research landing submission",
    );
    expect(updates).toContain("starting");
    expect(updates).toContain("waiting_for_sources");
    expect(updates).toContain("running");
    expect(pageState.researchLandingVisited).toBe("true");
    expect(pageState.researchSubmitted).toBe("true");
    expect(["button", "keyboard"]).toContain(pageState.lastSubmissionMethod);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("runs a follow-up ask from a finished research conversation after clearing the active Deep research composer chip", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
          scenario: "research-success",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        scenario: "research-success",
      }),
      headless: true,
    });

    await adapter.research(
      {
        jobId: "job-research-chip-followup",
        query: "finish a research run before switching back to chat",
        timeoutMs: 8_000,
      },
      () => {},
    );

    await adapter.getManagedPageForTest()!.evaluate(() => {
      const trigger = document.querySelector("#model-switcher-trigger");
      if (!(trigger instanceof HTMLElement)) {
        throw new Error("model trigger missing");
      }
      trigger.hidden = true;

      const chip = document.createElement("button");
      chip.type = "button";
      chip.textContent = "Deep research";
      chip.className = "__composer-pill";
      chip.setAttribute("aria-label", "Deep research, click to remove");
      chip.addEventListener("click", () => {
        chip.remove();
        trigger.hidden = false;
        document.body.dataset.selectedMode = "normal";
        const syncState = (
          window as Window & { syncSelectedState?: () => void }
        ).syncSelectedState;
        if (typeof syncState === "function") {
          syncState();
        }
      });

      const composer = document.querySelector("[data-testid='composer-root']");
      composer?.appendChild(chip);
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-chip-followup",
        prompt: "reply from a finished research thread",
        modelClass: "thinking",
        effort: "standard",
        timeoutMs: 7_000,
      },
      () => {},
    );

    expect(result.output_text).toContain(
      "Fake response for: reply from a finished research thread",
    );

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("clears blocking composer chips until the model picker returns before a normal ask", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
      }),
      headless: true,
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      const trigger = document.querySelector("#model-switcher-trigger");
      const composer = document.querySelector("[data-testid='composer-root']");
      if (
        !(trigger instanceof HTMLElement) ||
        !(composer instanceof HTMLElement)
      ) {
        throw new Error("composer setup missing");
      }

      trigger.hidden = true;
      const syncState = (window as Window & { syncSelectedState?: () => void })
        .syncSelectedState;
      const maybeRestoreTrigger = () => {
        if (
          composer.querySelectorAll("button[aria-label*='remove']").length === 0
        ) {
          trigger.hidden = false;
          if (typeof syncState === "function") {
            syncState();
          }
        }
      };

      for (const label of ["Deep research", "Web search", "Canvas"]) {
        const chip = document.createElement("button");
        chip.type = "button";
        chip.textContent = label;
        chip.className = "__composer-pill";
        chip.setAttribute("aria-label", `${label}, click to remove`);
        chip.addEventListener("click", () => {
          chip.remove();
          maybeRestoreTrigger();
        });
        composer.appendChild(chip);
      }
    });

    const prompt = "reply after chip clear";
    const result = await adapter.ask(
      {
        jobId: "job-ask-clears-blocking-composer-chips",
        prompt,
        modelClass: "thinking",
        effort: "standard",
        timeoutMs: 12_000,
      },
      () => {},
    );

    expect(result.output_text).toContain(`Fake response for: ${prompt}`);

    const pageState = await adapter.getManagedPageForTest()!.evaluate(() => ({
      remainingChips: document.querySelectorAll(
        "[data-testid='composer-root'] button[aria-label*='remove']",
      ).length,
      modelTriggerVisible:
        document.querySelector("#model-switcher-trigger") instanceof
          HTMLElement &&
        !(document.querySelector("#model-switcher-trigger") as HTMLElement)
          .hidden,
    }));

    expect(pageState.remainingChips).toBe(0);
    expect(pageState.modelTriggerVisible).toBe(true);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("keeps the selected thinking state while applying effort when the model picker hides", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
      }),
      headless: true,
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      const trigger = document.querySelector("#model-switcher-trigger");
      const syncState = (window as Window & { syncSelectedState?: () => void })
        .syncSelectedState;
      if (
        !(trigger instanceof HTMLElement) ||
        typeof syncState !== "function"
      ) {
        throw new Error("live model controls missing");
      }

      (
        window as Window & {
          syncSelectedState?: () => void;
          __tablinqOriginalSyncSelectedState__?: () => void;
        }
      ).__tablinqOriginalSyncSelectedState__ = syncState;

      (
        window as Window & {
          syncSelectedState?: () => void;
          __tablinqOriginalSyncSelectedState__?: () => void;
        }
      ).syncSelectedState = () => {
        (
          window as Window & {
            __tablinqOriginalSyncSelectedState__?: () => void;
          }
        ).__tablinqOriginalSyncSelectedState__?.();
        trigger.hidden = document.body.dataset.selectedModel === "thinking";
      };

      (
        window as Window & {
          syncSelectedState?: () => void;
        }
      ).syncSelectedState?.();
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-thinking-effort-hidden-model-picker",
        prompt: "reply after selecting thinking with a hidden model picker",
        modelClass: "thinking",
        effort: "standard",
        timeoutMs: 7_000,
      },
      () => {},
    );

    expect(result.output_text).toContain(
      "Fake response for: reply after selecting thinking with a hidden model picker",
    );

    const pageState = await adapter.getManagedPageForTest()!.evaluate(() => ({
      selectedModel: document.body.dataset.selectedModel,
      selectedEffort: document.body.dataset.selectedEffort,
      thinkingChipVisible:
        document.querySelector("#thinking-effort-trigger") instanceof
          HTMLElement &&
        !(document.querySelector("#thinking-effort-trigger") as HTMLElement)
          .hidden,
    }));

    expect(pageState.selectedModel).toBe("thinking");
    expect(pageState.selectedEffort).toBe("standard");
    expect(pageState.thinkingChipVisible).toBe(true);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails clearly when /deep-research landing submission never starts and does not emit recovery loops", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
          scenario: "research-deep-landing-submit-fail",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        scenario: "research-deep-landing-submit-fail",
      }),
      headless: true,
    });

    const states: string[] = [];
    await expect(
      adapter.research(
        {
          jobId: "job-research-deep-landing-submit-fail",
          query: "fail deep research landing submission",
          timeoutMs: 2_000,
        },
        () => {},
        (update) => {
          states.push(update.state);
        },
      ),
    ).rejects.toThrow(/deep-research but research never started/i);

    const pageState = await adapter.getManagedPageForTest()!.evaluate(() => ({
      researchLandingVisited: document.body.dataset.researchLandingVisited,
      researchSubmitted: document.body.dataset.researchSubmitted,
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
    }));

    expect(states).toEqual([]);
    expect(pageState.researchLandingVisited).toBe("true");
    expect(pageState.researchSubmitted).toBe("false");
    expect(["button", "keyboard"]).toContain(pageState.lastSubmissionMethod);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails research jobs with a research-specific error", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
          scenario: "research-failure",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        scenario: "research-failure",
      }),
      headless: true,
    });

    await expect(
      adapter.research(
        {
          jobId: "job-research-failure",
          query: "investigate flaky providers",
          timeoutMs: 5_000,
        },
        () => {},
      ),
    ).rejects.toThrow(/research job failed/i);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails classic research jobs with a research-specific error", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "research-failure",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        scenario: "research-failure",
      }),
      headless: true,
    });

    await expect(
      adapter.research(
        {
          jobId: "job-research-failure-classic",
          query: "investigate classic flaky providers",
          timeoutMs: 5_000,
        },
        () => {},
      ),
    ).rejects.toThrow(/research job failed/i);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("includes uploaded file names in classic research output", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "research-success",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        scenario: "research-success",
      }),
      headless: true,
    });

    const result = await adapter.research(
      {
        jobId: "job-research-success-classic",
        query: "summarize the upload",
        files: [fixture],
        timeoutMs: 5_000,
      },
      () => {},
    );

    expect(result.report_text).toContain("brief.txt");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails with an upload error when files are requested without an upload control", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, { missing: ["upload"] }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { missing: ["upload"] }),
      headless: true,
    });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-missing-upload",
          prompt: "summarize uploads",
          files: [fixture],
        },
        () => {},
      ),
    ).rejects.toThrow(/upload failed/i);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("falls back to keyboard submit when a composer plus button sits next to send", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "submit-upload-trap",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const result = await adapter.ask(
      {
        jobId: "job-ask-submit-upload-trap",
        prompt: "repeat attached file",
        files: [fixture],
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      uploadPopoverOpen: document.body.dataset.uploadPopoverOpen,
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
      uploadChips: document.querySelectorAll("[data-testid='upload-chip']")
        .length,
      plusPresent: Boolean(document.querySelector("#composer-plus-btn")),
    }));

    expect(result.output_text).toBe("Fake response for: repeat attached file");
    expect(state).toEqual({
      uploadPopoverOpen: "false",
      lastSubmissionMethod: "keyboard",
      uploadChips: 1,
      plusPresent: true,
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers when upload acknowledgment leaves a stale visible send control ahead of the active composer", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "submit-upload-stale-recover",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const result = await adapter.ask(
      {
        jobId: "job-ask-submit-upload-stale-recover",
        prompt: "repeat attached file",
        files: [fixture],
        timeoutMs: 10_000,
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
      staleSubmitClicks: document.body.dataset.staleSubmitClicks,
      keyboardSubmitReady: document.body.dataset.keyboardSubmitReady,
    }));

    expect(result.output_text).toBe("Fake response for: repeat attached file");
    expect(state).toEqual({
      lastSubmissionMethod: "button",
      staleSubmitClicks: "1",
      keyboardSubmitReady: "false",
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("waits for real submit readiness after upload acknowledgment", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "upload-processing-delay",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const startedAt = Date.now();
    const result = await adapter.ask(
      {
        jobId: "job-ask-upload-processing-delay",
        prompt: "repeat attached file after processing",
        files: [fixture],
        timeoutMs: 10_000,
      },
      () => {},
    );
    const elapsedMs = Date.now() - startedAt;
    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      uploadProcessing: document.body.dataset.uploadProcessing,
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
    }));

    expect(result.output_text).toBe(
      "Fake response for: repeat attached file after processing",
    );
    expect(elapsedMs).toBeGreaterThanOrEqual(1_800);
    expect(state).toEqual({
      uploadProcessing: "false",
      lastSubmissionMethod: "button",
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails clearly when upload acknowledgment never becomes send-ready", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "upload-processing-stuck",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-upload-processing-stuck",
          prompt: "repeat attached file",
          files: [fixture],
          timeoutMs: 1_500,
        },
        () => {},
      ),
    ).rejects.toThrow(/attachments are still processing/i);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("rejects files larger than 20 MiB before upload", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "oversize.bin");
    await writeFile(fixture, Buffer.alloc(MAX_UPLOAD_FILE_BYTES + 1));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
      }),
    );
    await adapter.open({ targetUrl: server.url, headless: true });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-oversize-upload",
          prompt: "repeat attached file",
          files: [fixture],
        },
        () => {},
      ),
    ).rejects.toThrow(/file exceeds 20 MiB/i);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails clearly when submit never becomes ready after upload acknowledgment", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "submit-never-ready",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-submit-upload-trap-fail",
          prompt: "repeat attached file",
          files: [fixture],
          timeoutMs: 1_500,
        },
        () => {},
      ),
    ).rejects.toThrow(/submit never became ready/i);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails cleanly when the harness never completes streaming", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, { scenario: "timeout" }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { scenario: "timeout" }),
      headless: true,
    });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-timeout",
          prompt: "never finish",
          timeoutMs: 600,
        },
        () => {},
      ),
    ).rejects.toThrow("timed out");

    const diagnostics = await adapter.diagnostics(path.join(root, "bundle"));
    expect(diagnostics.screenshot_path).toContain("failure.png");
    expect(diagnostics.dom_snapshot).toContain("dom_text");
    expect(diagnostics.aria_snapshot).toContain("role");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("continues ask streaming after a page reload", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, { scenario: "reload" }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { scenario: "reload" }),
      headless: true,
    });

    const states: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-reload",
        prompt: "survive reloads",
        timeoutMs: 5000,
      },
      () => {},
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.output_text).toBe("Fake response for: survive reloads");
    expect((await adapter.health()).reload_count).toBeGreaterThan(0);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers ask streaming after a browser disconnect", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
      }),
    );
    await adapter.open({ targetUrl: server.url, headless: true });

    const states: string[] = [];
    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-browser-disconnect",
        prompt: "recover after browser disconnect",
        timeoutMs: 7000,
      },
      async (chunk) => {
        if (!closeTriggered && chunk.kind === "delta") {
          closeTriggered = true;
          await adapter.getManagedPageForTest()!.context().browser()!.close();
        }
      },
      (update) => {
        states.push(update.state);
      },
    );

    const result = await request;
    expect(result.output_text).toBe(
      "Fake response for: recover after browser disconnect",
    );
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers a follow-up ask in the same conversation without replaying the previous reply", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      scenario: "followup-stale",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const first = await adapter.ask(
      {
        jobId: "job-ask-followup-recovery-first",
        prompt: "hello gpt",
      },
      () => {},
    );

    const states: string[] = [];
    const secondChunks: string[] = [];
    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-followup-recovery-second",
        prompt: "recover the follow-up ask",
        conversationId: first.conversation_id,
        timeoutMs: 7000,
      },
      async (chunk) => {
        if (chunk.kind === "delta") {
          secondChunks.push(chunk.text);
          if (!closeTriggered) {
            closeTriggered = true;
            await adapter.getManagedPageForTest()!.context().browser()!.close();
          }
        }
      },
      (update) => {
        states.push(update.state);
      },
    );

    const second = await request;
    expect(second.conversation_id).toBe(first.conversation_id);
    expect(second.output_text).toBe(
      "Fake response for: recover the follow-up ask",
    );
    expect(secondChunks.join("")).toBe(second.output_text);
    expect(secondChunks.join("")).not.toContain(first.output_text);
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers a follow-up ask from the ask baseline conversation when the cached target URL is stale", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      scenario: "followup-stale",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const first = await adapter.ask(
      {
        jobId: "job-ask-followup-baseline-first",
        prompt: "hello gpt",
      },
      () => {},
    );
    await adapter.getManagedPageForTest()!.evaluate(() => {
      document.body.dataset.streamState = "done";
      const latest = Array.from(
        document.querySelectorAll("[data-message-author-role='assistant']"),
      ).at(-1);
      if (latest instanceof HTMLElement) {
        latest.setAttribute("data-stream-state", "done");
      }
    });

    const adapterWithInternals = adapter as any;
    adapterWithInternals.currentTargetUrl = server.url;

    const chunks: string[] = [];
    const result = await adapterWithInternals.reconcileAskAfterInterruption(
      adapter.getManagedPageForTest()!,
      {
        jobId: "job-ask-followup-baseline-recover",
        prompt: "hello gpt",
        timeoutMs: 7_000,
      },
      {
        emittedText: "",
        nextSeq: 0,
        doneStreak: 0,
        knownConversationId: null,
        knownMessageId: null,
        askBaseline: {
          assistantMessageCount: 0,
          conversationId: first.conversation_id,
          messageId: null,
          text: "",
        },
        pendingTextReconciliation: false,
        recoveryAttempts: 0,
        lastStatusAt: 0,
      },
      (chunk: any) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
      () => {},
      "recovered the interrupted ask",
      Date.now() + 7_000,
    );

    expect(result.result?.conversation_id).toBe(first.conversation_id);
    expect(result.result?.output_text).toBe(first.output_text);
    expect(chunks.join("")).toBe(first.output_text);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers from a temporary streaming stall without replaying the prompt", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "stall-recover",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { scenario: "stall-recover" }),
      headless: true,
    });

    const chunks: string[] = [];
    const states: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-stall-recover",
        prompt: "resume after a temporary stall",
        timeoutMs: 6000,
      },
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.output_text).toBe(
      "Fake response for: resume after a temporary stall",
    );
    expect(chunks.join("")).toBe(result.output_text);
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers ask when the live page is transiently ambiguous during stall recovery", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
          scenario: "stall-recover-transient",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        scenario: "stall-recover-transient",
      }),
      headless: true,
    });

    const chunks: string[] = [];
    const states: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-stall-recover-transient",
        prompt: "recover after a transient live page",
        files: [fixture],
        timeoutMs: 7000,
      },
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.output_text).toBe(
      "Fake response for: recover after a transient live page",
    );
    expect(result.artifacts).toHaveLength(1);
    expect(chunks.join("")).toBe(result.output_text);
    expect(result.current_url).toContain(`/c/${result.conversation_id}`);
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers a final result that completed upstream while disconnected", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "recover-complete",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { scenario: "recover-complete" }),
      headless: true,
    });

    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-recover-complete",
        prompt: "complete while disconnected",
        timeoutMs: 7000,
      },
      async (chunk) => {
        if (!closeTriggered && chunk.kind === "delta") {
          closeTriggered = true;
          await adapter.getManagedPageForTest()!.context().browser()!.close();
        }
      },
      () => {},
    );

    const result = await request;
    expect(result.output_text).toBe(
      "Fake response for: complete while disconnected",
    );
    expect(result.current_url).toContain(`/c/${result.conversation_id}`);
    expect(conversationIdFromUrl(result.current_url)).toBe(
      result.conversation_id,
    );

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers a final ask after transient placeholder drift stabilizes", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "recover-done-transient",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        scenario: "recover-done-transient",
      }),
      headless: true,
    });

    const states: string[] = [];
    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-recover-done-transient",
        prompt: "Reply with exactly: ASK_OK",
        timeoutMs: 7000,
      },
      async (chunk) => {
        if (!closeTriggered && chunk.kind === "delta") {
          closeTriggered = true;
          await adapter.getManagedPageForTest()!.context().browser()!.close();
        }
      },
      (update) => {
        states.push(update.state);
      },
    );

    const result = await request;
    expect(result.output_text).toBe(
      "Fake response for: Reply with exactly: ASK_OK",
    );
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers when the assistant message id rotates to a later response in the same conversation", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "recover-message-successor",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        scenario: "recover-message-successor",
      }),
      headless: true,
    });

    const states: string[] = [];
    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-recover-message-successor",
        prompt: "recover after assistant message rotation",
        timeoutMs: 7000,
      },
      async (chunk) => {
        if (!closeTriggered && chunk.kind === "delta") {
          closeTriggered = true;
          await adapter.getManagedPageForTest()!.context().browser()!.close();
        }
      },
      (update) => {
        states.push(update.state);
      },
    );

    const result = await request;
    expect(result.output_text).toBe(
      "Fake response for: recover after assistant message rotation",
    );
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers an in-flight ask mismatch that later converges on the same message", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "recover-running-transient",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        scenario: "recover-running-transient",
      }),
      headless: true,
    });

    const states: string[] = [];
    const chunks: string[] = [];
    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-recover-running-transient",
        prompt: "recover after placeholder drift",
        timeoutMs: 7000,
      },
      async (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
          if (!closeTriggered) {
            closeTriggered = true;
            await adapter.getManagedPageForTest()!.context().browser()!.close();
          }
        }
      },
      (update) => {
        states.push(update.state);
      },
    );

    const result = await request;
    expect(result.output_text).toBe(
      "Fake response for: recover after placeholder drift",
    );
    expect(chunks.join("")).toBe(result.output_text);
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers when the same assistant message rewrites provisional thinking text into the final answer", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "recover-same-message-rewrite",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        scenario: "recover-same-message-rewrite",
      }),
      headless: true,
    });

    const states: string[] = [];
    const chunks: string[] = [];
    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-recover-same-message-rewrite",
        prompt: "recover after thinking rewrite",
        timeoutMs: 7_000,
      },
      async (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
        if (chunk.kind === "delta" && !closeTriggered) {
          closeTriggered = true;
          await adapter.getManagedPageForTest()!.context().browser()!.close();
        }
      },
      (update) => {
        states.push(update.state);
      },
    );

    const result = await request;
    expect(result.output_text).toBe(
      "Fake response for: recover after thinking rewrite",
    );
    expect(chunks.join("")).not.toContain("Thought for 7s");
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers a finished ask when the saved emitted text is only transient thinking status", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
      }),
    );
    await adapter.open({
      targetUrl: server.url,
      headless: true,
    });

    const prompt = "recover after transient thinking status";
    const initial = await adapter.ask(
      {
        jobId: "job-ask-transient-thinking-status-initial",
        prompt,
        timeoutMs: 7_000,
      },
      () => undefined,
      () => undefined,
    );

    const recovered = await adapter.recoverAsk(
      {
        jobId: "job-ask-transient-thinking-status-recover",
        prompt,
        conversationId: initial.conversation_id,
        timeoutMs: 7_000,
      },
      {
        emittedText: "Thought for 7s",
        nextSeq: 0,
      },
      () => undefined,
      () => undefined,
    );

    expect(recovered.output_text).toBe(initial.output_text);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers a same-message ask when the final live DOM becomes rich markdown", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "recover-rich-markdown",
    });
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({
      targetUrl,
      headless: true,
    });

    const states: string[] = [];
    let closeTriggered = false;
    const result = await adapter.ask(
      {
        jobId: "job-ask-recover-rich-markdown",
        prompt: "recover copyable block after interruption",
        timeoutMs: 7000,
      },
      async (chunk) => {
        if (chunk.kind === "delta" && !closeTriggered) {
          closeTriggered = true;
          await adapter.getManagedPageForTest()!.context().browser()!.close();
        }
      },
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.output_text).toBe(
      [
        "```",
        "Summarize release notes into a concise update with risks and actions.",
        "```",
        "",
        "**Outline**",
        "",
        "1. Goal",
        "2. Inputs",
        "3. Constraints",
        "4. Output",
      ].join("\n"),
    );
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails when an interrupted ask cannot be reconciled against the live conversation", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "recover-mismatch",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { scenario: "recover-mismatch" }),
      headless: true,
    });

    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-recover-mismatch",
        prompt: "fail recovery on mismatched output",
        timeoutMs: 7000,
      },
      async (chunk) => {
        if (!closeTriggered && chunk.kind === "delta") {
          closeTriggered = true;
          await adapter.getManagedPageForTest()!.context().browser()!.close();
        }
      },
      () => {},
    );

    await expect(request).rejects.toThrow(/could not be reconciled/i);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails when interrupted ask recovery lands on a different conversation", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "recover-wrong-conversation",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        scenario: "recover-wrong-conversation",
      }),
      headless: true,
    });

    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-recover-wrong-conversation",
        prompt: "fail recovery on wrong conversation",
        timeoutMs: 7000,
      },
      async (chunk) => {
        if (!closeTriggered && chunk.kind === "delta") {
          closeTriggered = true;
          await adapter.getManagedPageForTest()!.context().browser()!.close();
        }
      },
      () => {},
    );

    await expect(request).rejects.toThrow(/could not be reconciled/i);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers research streaming when the live conversation moves to /c/<id>", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
          scenario: "research-success",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        scenario: "research-success",
      }),
      headless: true,
    });

    const states: string[] = [];
    let closeTriggered = false;
    const result = await adapter.research(
      {
        jobId: "job-research-recover-current-url",
        query: "recover research after browser disconnect",
        timeoutMs: 7_000,
      },
      async (update) => {
        if (
          !closeTriggered &&
          (update.status === "waiting_for_sources" ||
            (update.status === "running" && Boolean(update.chunk_text?.length)))
        ) {
          closeTriggered = true;
          await adapter.getManagedPageForTest()!.context().browser()!.close();
        }
      },
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.report_text).toContain(
      "Research report for: recover research after browser disconnect",
    );
    expect(result.current_url).toContain(`/c/${result.conversation_id}`);
    expect(conversationIdFromUrl(result.current_url)).toBe(
      result.conversation_id,
    );
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers a live research job after the client reconnects to an attached browser", async () => {
    const server = await startTrackedFakeChatGptServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "research-success",
    });
    const attached = await startTrackedAttachableBrowser(targetUrl);

    await withAttachedBrowser(attached.endpoint, async (browser) => {
      const page = browser.contexts()[0]!.pages()[0]!;
      await page.waitForLoadState("domcontentloaded");
      await page.locator("#deep-research-nav").click();
      await page
        .locator("#prompt-textarea")
        .fill("reconnect to a running deep research job");
      await page.locator("[data-testid='send-button']").click();
      await page
        .frameLocator("#deep-research-sandbox")
        .getByRole("button", { name: "Start" })
        .click();
      await page.waitForTimeout(250);
    });

    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const states: string[] = [];
    const updates: string[] = [];
    const result = await adapter.recoverResearch(
      {
        jobId: "job-research-recover-attached-client",
        query: "reconnect to a running deep research job",
        timeoutMs: 7_000,
      },
      { emittedText: "", nextSeq: 0 },
      (update) => {
        updates.push(update.status);
      },
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.report_text).toContain(
      "Research report for: reconnect to a running deep research job",
    );
    expect(updates).toContain("running");
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");
    expect(result.current_url).toContain(`/c/${result.conversation_id}`);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  }, 30_000);

  it("recovers a running deep research job from the active conversation tab when another attached tab is selected", async () => {
    const server = await startTrackedFakeChatGptServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "research-success",
    });
    const attached = await startTrackedAttachableBrowser(targetUrl);

    await withAttachedBrowser(attached.endpoint, async (browser) => {
      const context = browser.contexts()[0]!;
      const rootPage = context.pages()[0]!;
      await rootPage.waitForLoadState("domcontentloaded");
      const researchPage = await context.newPage();
      await researchPage.goto(targetUrl, { waitUntil: "domcontentloaded" });
      await researchPage.locator("#deep-research-nav").click();
      await researchPage
        .locator("#prompt-textarea")
        .fill("recover the active research tab after a client reconnect");
      await researchPage.locator("[data-testid='send-button']").click();
      await researchPage
        .frameLocator("#deep-research-sandbox")
        .getByRole("button", { name: "Start" })
        .click();
      await rootPage.bringToFront();
      await researchPage.waitForTimeout(250);
    });

    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const states: string[] = [];
    const result = await adapter.recoverResearch(
      {
        jobId: "job-research-recover-active-tab",
        query: "recover the active research tab after a client reconnect",
        timeoutMs: 7_000,
      },
      { emittedText: "", nextSeq: 0 },
      () => {},
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.report_text).toContain(
      "Research report for: recover the active research tab after a client reconnect",
    );
    expect(result.current_url).toContain(`/c/${result.conversation_id}`);
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  }, 30_000);

  it("recovers a running deep research job after the attached research tab reloads", async () => {
    const server = await startTrackedFakeChatGptServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "research-reload",
    });
    const attached = await startTrackedAttachableBrowser(targetUrl);

    await withAttachedBrowser(attached.endpoint, async (browser) => {
      const page = browser.contexts()[0]!.pages()[0]!;
      await page.waitForLoadState("domcontentloaded");
      await page.locator("#deep-research-nav").click();
      await page
        .locator("#prompt-textarea")
        .fill("recover a deep research run after a tab reload");
      await page.locator("[data-testid='send-button']").click();
      await page
        .frameLocator("#deep-research-sandbox")
        .getByRole("button", { name: "Start" })
        .click();
      await page.waitForTimeout(250);
    });

    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const updates: string[] = [];
    const states: string[] = [];
    const result = await adapter.recoverResearch(
      {
        jobId: "job-research-recover-reload",
        query: "recover a deep research run after a tab reload",
        timeoutMs: 7_000,
      },
      { emittedText: "", nextSeq: 0 },
      (update) => {
        updates.push(update.status);
      },
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.report_text).toContain(
      "Research report for: recover a deep research run after a tab reload",
    );
    expect(updates).toContain("running");
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  }, 30_000);

  it("streams ask responses to completion in attach mode", async () => {
    const server = await startTrackedFakeChatGptServer();
    const adapter = await createTrackedAdapterForMode("attach", server.url);
    await adapter.open({ targetUrl: server.url, headless: true });

    const chunks: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-success-attach",
        prompt: "hello from attach mode",
      },
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );

    expect(chunks.join("")).toBe("Fake response for: hello from attach mode");
    expect(result.output_text).toBe(
      "Fake response for: hello from attach mode",
    );
    expect(result.conversation_id).toMatch(/^conv-/);
    expect(result.message_id).toMatch(/^msg-/);
    expect(result.current_url).toContain(`/c/${result.conversation_id}`);
    expect(conversationIdFromUrl(result.current_url)).toBe(
      result.conversation_id,
    );

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("drives the live-like harness through ask and artifact persistence in attach mode", async () => {
    const server = await startTrackedFakeChatGptServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      initialModel: "thinking",
      initialEffort: "extended",
    });
    const adapter = await createTrackedAdapterForMode("attach", targetUrl);
    const fixtureRoot = await mkdtemp(
      path.join(os.tmpdir(), "tablinq-browser-"),
    );
    const fixture = path.join(fixtureRoot, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    await adapter.open({ targetUrl, headless: true });

    const result = await adapter.ask(
      {
        jobId: "job-ask-live-attach",
        prompt: "configure attach live",
        mode: "research",
        modelClass: "thinking",
        effort: "extended",
        files: [fixture],
      },
      () => {},
    );
    const artifactBody = await readFile(
      result.artifacts[0]!.storage_path,
      "utf8",
    );

    expect(result.output_text).toContain(
      "Research report for: configure attach live",
    );
    expect(result.artifacts).toHaveLength(1);
    expect(artifactBody).toContain("Model: thinking");
    expect(artifactBody).toContain("Effort: extended");
    expect(artifactBody).toContain("Uploads: brief.txt");
    expect(result.current_url).toContain(`/c/${result.conversation_id}`);
    expect(conversationIdFromUrl(result.current_url)).toBe(
      result.conversation_id,
    );

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("waits for a stabilized final ask snapshot in attach mode", async () => {
    const server = await startTrackedFakeChatGptServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "done-early",
    });
    const adapter = await createTrackedAdapterForMode("attach", targetUrl);
    const fixtureRoot = await mkdtemp(
      path.join(os.tmpdir(), "tablinq-browser-"),
    );
    const fixture = path.join(fixtureRoot, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    await adapter.open({ targetUrl, headless: true });

    const chunks: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-done-early-attach",
        prompt: "Reply with exactly: ASK_OK",
        files: [fixture],
      },
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );

    expect(result.output_text).toBe(
      "Fake response for: Reply with exactly: ASK_OK",
    );
    expect(chunks.join("")).toBe(result.output_text);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails with a capability-missing error in attach mode when search mode is unavailable", async () => {
    const server = await startTrackedFakeChatGptServer();
    const targetUrl = withHarnessUrl(server.url, { missing: ["search"] });
    const adapter = await createTrackedAdapterForMode("attach", targetUrl);
    await adapter.open({ targetUrl, headless: true });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-missing-search-attach",
          prompt: "search in attach mode",
          mode: "search",
        },
        () => {},
      ),
    ).rejects.toThrow(/capability missing: search mode/i);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("fails cleanly when the harness never completes streaming in attach mode", async () => {
    const server = await startTrackedFakeChatGptServer();
    const targetUrl = withHarnessUrl(server.url, { scenario: "timeout" });
    const adapter = await createTrackedAdapterForMode("attach", targetUrl);
    const bundleRoot = await mkdtemp(
      path.join(os.tmpdir(), "tablinq-browser-"),
    );
    await adapter.open({ targetUrl, headless: true });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-timeout-attach",
          prompt: "never finish in attach mode",
          timeoutMs: 600,
        },
        () => {},
      ),
    ).rejects.toThrow("timed out");

    const diagnostics = await adapter.diagnostics(
      path.join(bundleRoot, "bundle"),
    );
    expect(diagnostics.screenshot_path).toContain("failure.png");
    expect(diagnostics.dom_snapshot).toContain("dom_text");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("classifies provider generation failures without clicking Retry", async () => {
    const server = await startTrackedFakeChatGptServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "provider-generation-failure-twice",
    });
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    let error: any = null;
    try {
      await adapter.ask(
        {
          jobId: "job-ask-provider-generation-failure",
          prompt: "reason carefully about this tradeoff",
        },
        () => {},
      );
    } catch (caught) {
      error = caught;
    }

    expect(error).toBeTruthy();
    expect(error.code).toBe("provider_generation_failed_nonretryable");
    expect(error.retryVisible).toBe(true);
    expect(await adapter.getManagedPageForTest()!.evaluate(() => ({
      askFailureCode: document.body.dataset.askFailureCode ?? null,
      retryClicks: document.body.dataset.retryClicks ?? "0",
    }))).toEqual({
      askFailureCode: "provider_generation_failed_nonretryable",
      retryClicks: "0",
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("continues ask streaming after a page reload in attach mode", async () => {
    const server = await startTrackedFakeChatGptServer();
    const targetUrl = withHarnessUrl(server.url, { scenario: "reload" });
    const adapter = await createTrackedAdapterForMode("attach", targetUrl);
    await adapter.open({ targetUrl, headless: true });

    const states: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-reload-attach",
        prompt: "survive attach reloads",
        timeoutMs: 5000,
      },
      () => {},
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.output_text).toBe(
      "Fake response for: survive attach reloads",
    );
    expect((await adapter.health()).reload_count).toBeGreaterThan(0);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("recovers ask streaming after a browser disconnect in attach mode", async () => {
    const server = await startTrackedFakeChatGptServer();
    const adapter = await createTrackedAdapterForMode("attach", server.url);
    await adapter.open({ targetUrl: server.url, headless: true });

    const states: string[] = [];
    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-browser-disconnect-attach",
        prompt: "recover after attach disconnect",
        timeoutMs: 7000,
      },
      async (chunk) => {
        if (!closeTriggered && chunk.kind === "delta") {
          closeTriggered = true;
          await adapter.getManagedPageForTest()!.context().browser()!.close();
        }
      },
      (update) => {
        states.push(update.state);
      },
    );

    const result = await request;
    expect(result.output_text).toBe(
      "Fake response for: recover after attach disconnect",
    );
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("runs research jobs with progress updates in attach mode", async () => {
    const server = await startTrackedFakeChatGptServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "research-success",
    });
    const adapter = await createTrackedAdapterForMode("attach", targetUrl);
    const fixtureRoot = await mkdtemp(
      path.join(os.tmpdir(), "tablinq-browser-"),
    );
    const fixture = path.join(fixtureRoot, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    await adapter.open({ targetUrl, headless: true });

    const updates: string[] = [];
    const result = await adapter.research(
      {
        jobId: "job-research-success-attach",
        query: "investigate attach launch risks",
        files: [fixture],
        timeoutMs: 5_000,
      },
      (update) => {
        updates.push(update.status);
      },
    );

    expect(updates).toContain("starting");
    expect(updates).toContain("waiting_for_sources");
    expect(updates).toContain("running");
    expect(result.report_text).toContain(
      "Research report for: investigate attach launch risks",
    );
    expect(result.artifacts).toHaveLength(1);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  }, 30_000);

  it("fails research jobs with a research-specific error in attach mode", async () => {
    const server = await startTrackedFakeChatGptServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "research-failure",
    });
    const adapter = await createTrackedAdapterForMode("attach", targetUrl);
    await adapter.open({ targetUrl, headless: true });

    await expect(
      adapter.research(
        {
          jobId: "job-research-failure-attach",
          query: "investigate attach flaky providers",
          timeoutMs: 5_000,
        },
        () => {},
      ),
    ).rejects.toThrow(/research job failed/i);

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  }, 30_000);
});

describe("browser launch config", () => {
  it("prefers an explicit browser path", () => {
    const config = resolveBrowserLaunchConfig(
      "win32",
      "C:\\Browsers\\chrome.exe",
    );
    expect(config).toEqual({
      browserKind: "chrome",
      executablePath: "C:\\Browsers\\chrome.exe",
    });
  });

  it("prefers Chrome before Edge on Windows", async () => {
    const root = await mkdtemp(
      path.join(os.tmpdir(), "tablinq-browser-detect-"),
    );
    const localAppData = path.join(root, "LocalAppData");
    const programFiles = path.join(root, "ProgramFiles");
    const chromePath = path.join(
      localAppData,
      "Google",
      "Chrome",
      "Application",
      "chrome.exe",
    );
    const edgePath = path.join(
      programFiles,
      "Microsoft",
      "Edge",
      "Application",
      "msedge.exe",
    );

    await mkdir(path.dirname(chromePath), { recursive: true });
    await mkdir(path.dirname(edgePath), { recursive: true });
    await writeFile(chromePath, "", "utf8");
    await writeFile(edgePath, "", "utf8");

    const previousLocalAppData = process.env.LOCALAPPDATA;
    const previousProgramFiles = process.env.ProgramFiles;
    const previousProgramFilesX86 = process.env["ProgramFiles(x86)"];
    const previousBrowserPath = process.env.TABLINQ_BROWSER_PATH;

    process.env.LOCALAPPDATA = localAppData;
    process.env.ProgramFiles = programFiles;
    process.env["ProgramFiles(x86)"] = programFiles;
    delete process.env.TABLINQ_BROWSER_PATH;

    try {
      expect(detectBrowserExecutable("win32")).toBe(chromePath);
      expect(resolveBrowserLaunchConfig("win32")).toEqual({
        browserKind: "chrome",
        executablePath: chromePath,
      });
    } finally {
      process.env.LOCALAPPDATA = previousLocalAppData;
      process.env.ProgramFiles = previousProgramFiles;
      process.env["ProgramFiles(x86)"] = previousProgramFilesX86;
      if (previousBrowserPath === undefined) {
        delete process.env.TABLINQ_BROWSER_PATH;
      } else {
        process.env.TABLINQ_BROWSER_PATH = previousBrowserPath;
      }
    }
  });

  it("allocates unique artifact paths without overwriting existing files", async () => {
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-artifacts-"));
    const first = await chooseUniqueArtifactPath(root, "report.txt");
    await writeFile(first, "first", "utf8");
    const second = await chooseUniqueArtifactPath(root, "report.txt");

    expect(path.basename(first)).toBe("report.txt");
    expect(path.basename(second)).toBe("report-2.txt");
  });
});
