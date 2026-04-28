import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { JSDOM } from "jsdom";
import {
  BrowserAdapter,
  adaptivePollIntervalMs,
  chooseUniqueArtifactPath,
  conversationIdFromUrl,
  deepResearchStartDelayMs,
  detectBrowserExecutable,
  highRiskActionCooldownMs,
  interactionJitterMs,
  MAX_UPLOAD_FILE_BYTES,
  noteProviderPressure,
  noteProviderPressureFromText,
  pacingRandomValueFromSeed,
  providerPressureCooldownMs,
  resetSessionRhythmForTest,
  resolveConversationUrl,
  resolveBrowserLaunchConfig,
  uploadAckTimeoutMsForFiles,
  workspaceKeyFromUrl,
} from "../src/browserAdapter";
import { applyAskSnapshot } from "../src/execution/streaming";
import { loadPageAdapterSource } from "../src/assets";
import {
  scrollToBottomIfNeeded,
  waitForInteractionSurface,
  waitForQuietInteractionSurface,
} from "../src/pageRuntime";
import {
  createIsolatedTempDir,
  listAttachedPages,
  withAttachedBrowser,
  withHarnessUrl,
} from "./browser-adapter.integration.helpers";
import {
  expectHarnessCapabilityMatrix,
  expectManagedHarnessOpenSnapshot,
  openManagedHarnessAdapter,
  openTrackedManagedAdapter,
} from "./helpers/browserFlow";
import { installBrowserAdapterHarness } from "./helpers/browserAdapterHarness";

const harness = installBrowserAdapterHarness({ resetSessionRhythm: true });

describe("browser adapter foundation: session and bootstrap", () => {
  it("launches a managed browser and opens the fake harness", async () => {
    const server = await harness.startServer();
    const { adapter, snapshot } = await openTrackedManagedAdapter(
      harness,
      server.url,
    );

    expectManagedHarnessOpenSnapshot(snapshot, server.url);

    await adapter.shutdown();
  });

  it("keeps health and snapshot status fields aligned except title refresh", async () => {
    const server = await harness.startServer();
    const { adapter } = await openTrackedManagedAdapter(harness, server.url);

    const health = await adapter.health();
    const snapshot = await adapter.snapshot();

    expect({
      ...health,
      page_title: null,
    }).toEqual(snapshot);

    await adapter.shutdown();
  });

  it("disables the Deep Research start delay under vitest by default", () => {
    expect(process.env.VITEST).toBe("true");
    expect(deepResearchStartDelayMs()).toBe(0);
  });

  it("reuses the managed page on repeated open", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("reuses the persistent profile across adapter restarts", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const profileDir = path.join(root, "profile");
    const downloadsDir = path.join(root, "downloads");

    const first = harness.trackAdapter(
      new BrowserAdapter({
        profileDir,
        downloadsDir,
        defaultTargetUrl: server.url,
      }),
    );
    await first.open({ targetUrl: server.url, headless: true });
    await first.shutdown();

    const second = harness.trackAdapter(
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
  });

  it("reconnects after the managed tab is closed", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("captures screenshots on demand", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("attaches to an existing browser session", async () => {
    const server = await harness.startServer();
    const attached = await harness.startAttachableBrowser(server.url);
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("reuses an existing ChatGPT tab in attach mode", async () => {
    const server = await harness.startServer();
    const attached = await harness.startAttachableBrowser(server.url);
    const before = await listAttachedPages(attached.endpoint);
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("opens a new target tab when none exists in attach mode", async () => {
    const server = await harness.startServer();
    const attached = await harness.startAttachableBrowser("about:blank");
    const before = await listAttachedPages(attached.endpoint);
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("recovers after the attached target tab is closed", async () => {
    const server = await harness.startServer();
    const attached = await harness.startAttachableBrowser(server.url);
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("reopens the attached ChatGPT tab and completes ask after the target tab is closed", async () => {
    const server = await harness.startServer();
    const attached = await harness.startAttachableBrowser(server.url);
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("captures diagnostics screenshots in attach mode", async () => {
    const server = await harness.startServer();
    const attached = await harness.startAttachableBrowser(server.url);
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("keeps active tab metadata for attach mode", async () => {
    const server = await harness.startServer();
    const attached = await harness.startAttachableBrowser(server.url);
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("waits briefly for extension readiness after attach", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, { variant: "menu" });
    const attached = await harness.startAttachableBrowser(targetUrl);
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

    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("switches to the matching attached tab that has a working extension report", async () => {
    const server = await harness.startServer();
    const attached = await harness.startAttachableBrowser(server.url);
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

    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("closes stale attached ChatGPT tabs after selecting a healthy target tab", async () => {
    const server = await harness.startServer();
    const attached = await harness.startAttachableBrowser(server.url);
    await withAttachedBrowser(attached.endpoint, async (browser) => {
      const context = browser.contexts()[0]!;
      const stale = context.pages()[0]!;
      await stale.evaluate(() => {
        document.body.innerHTML = "<main>stale tab</main>";
      });

      const healthy = await context.newPage();
      await healthy.goto(server.url, { waitUntil: "domcontentloaded" });
    });

    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });
});
