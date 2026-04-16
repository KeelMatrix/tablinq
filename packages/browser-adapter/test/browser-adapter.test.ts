import { mkdir, mkdtemp, readFile, stat, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { spawn, type ChildProcess } from "node:child_process";
import { chromium } from "playwright-core";
import {
  BrowserAdapter,
  chooseUniqueArtifactPath,
  conversationIdFromUrl,
  detectBrowserExecutable,
  resolveConversationUrl,
  resolveBrowserLaunchConfig,
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
      | "submit-upload-trap"
      | "submit-upload-trap-no-keyboard"
      | "followup-rich-text"
      | "followup-stale"
      | "done-early"
      | "stall-recover"
      | "stall-recover-transient"
      | "recover-complete"
      | "recover-mismatch"
      | "research-success"
      | "research-failure";
    missing?: string[];
    conversationStyle?: "query" | "path";
    initialModel?: "instant" | "thinking";
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
    ["aria fallback", "aria"],
    ["live-like", "live"],
  ] as const)(
    "captures a full capability matrix for the %s harness",
    async (_label, variant) => {
      const server = await startTrackedFakeChatGptServer();
      const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
      const adapter = trackAdapter(
        new BrowserAdapter({
          profileDir: path.join(root, "profile"),
          downloadsDir: path.join(root, "downloads"),
          defaultTargetUrl: withHarnessUrl(server.url, { variant }),
        }),
      );

      const snapshot = await adapter.open({
        targetUrl: withHarnessUrl(server.url, { variant }),
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
    const attached = await startTrackedAttachableBrowser(server.url);
    await withAttachedBrowser(attached.endpoint, async (browser) => {
      const page = browser.contexts()[0]!.pages()[0]!;
      await page.evaluate(() => {
        const controls = Array.from(
          document.querySelectorAll(
            "[data-slot='composer'], [data-action='submit'], [data-slot='model-picker'], [data-slot='effort-toggle'], [data-slot='upload-entry']",
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
        defaultTargetUrl: server.url,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );

    const snapshot = await adapter.open({
      targetUrl: server.url,
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
      (chunk) => {
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
    const targetUrl = withHarnessUrl(server.url, { scenario: "followup-stale" });
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
    const targetUrl = withHarnessUrl(server.url, { scenario: "followup-stale" });
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

  it("selects model, effort, and search mode on the classic harness", async () => {
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

    await adapter.ask(
      {
        jobId: "job-ask-configured-classic",
        prompt: "configure classic",
        mode: "search",
        modelClass: "instant",
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
      model: "instant",
      effort: "extended",
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("selects renamed controls on the aria harness", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const adapter = trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, { variant: "aria" }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { variant: "aria" }),
      headless: true,
    });

    await adapter.ask(
      {
        jobId: "job-ask-configured-aria",
        prompt: "configure aria",
        mode: "research",
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
      mode: "research",
      model: "thinking",
      effort: "extended",
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("keeps direct model buttons working on the classic harness", async () => {
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

    await adapter.ask(
      {
        jobId: "job-ask-direct-model-buttons",
        prompt: "use classic direct buttons",
        modelClass: "instant",
        effort: "standard",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
    }));

    expect(state).toEqual({
      model: "instant",
      effort: "standard",
    });

    await adapter.shutdown();
    await stopFakeChatGptServer(server.process);
  });

  it("selects a model from a menu picker and then applies thinking effort", async () => {
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
        jobId: "job-ask-menu-model-picker",
        prompt: "configure menu picker",
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
      /capability missing: model class; requested=instant; discovered=thinking=\[gpt-5.4 thinking\]/i,
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
      /capability missing: thinking effort; requested=extended; discovered=none/i,
    );

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
        modelClass: "instant",
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
      sendHidden: false,
      stopHidden: true,
    });
    expect(result.output_text).toBe("Fake response for: configure live");
    expect(result.artifacts).toHaveLength(1);
    expect(artifactBody).toContain("Mode: research");
    expect(artifactBody).toContain("Model: instant");
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
      targetUrl: withHarnessUrl(server.url, { scenario: "research-success" }),
      headless: true,
    });

    const updates: string[] = [];
    const result = await adapter.research(
      {
        jobId: "job-research-success",
        query: "investigate launch risks",
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
      "Research report for: investigate launch risks",
    );
    expect(result.artifacts).toHaveLength(1);

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
          scenario: "research-failure",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { scenario: "research-failure" }),
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
      uploadChips: document.querySelectorAll("[data-testid='upload-chip']").length,
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

  it("fails fast when no submit action starts after click and keyboard fallback", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "submit-upload-trap-no-keyboard",
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
        },
        () => {},
      ),
    ).rejects.toThrow(/submit failed/i);

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
    expect(diagnostics.dom_snapshot).toContain("body_excerpt");
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
    const targetUrl = withHarnessUrl(server.url, { scenario: "followup-stale" });
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

  it("recovers research streaming when the live conversation moves to /c/<id>", async () => {
    const server = await startTrackedFakeChatGptServer();
    const root = await mkdtemp(path.join(os.tmpdir(), "tablinq-browser-"));
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
      targetUrl: withHarnessUrl(server.url, { scenario: "research-success" }),
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
            (update.status === "running" &&
              Boolean(update.chunk_text?.length)))
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
    const targetUrl = withHarnessUrl(server.url, { variant: "live" });
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
        modelClass: "instant",
        effort: "extended",
        files: [fixture],
      },
      () => {},
    );
    const artifactBody = await readFile(
      result.artifacts[0]!.storage_path,
      "utf8",
    );

    expect(result.output_text).toBe("Fake response for: configure attach live");
    expect(result.artifacts).toHaveLength(1);
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

    expect(result.output_text).toBe("Fake response for: Reply with exactly: ASK_OK");
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
    expect(diagnostics.dom_snapshot).toContain("body_excerpt");

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
  });

  it("fails research jobs with a research-specific error in attach mode", async () => {
    const server = await startTrackedFakeChatGptServer();
    const targetUrl = withHarnessUrl(server.url, {
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
  });
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
