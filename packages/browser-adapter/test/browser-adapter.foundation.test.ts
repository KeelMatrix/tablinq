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
  providerPressureCooldownMs,
  resetProviderPressureForTest,
  resolveConversationUrl,
  resolveBrowserLaunchConfig,
  uploadAckTimeoutMsForFiles,
  workspaceKeyFromUrl,
} from "../src/browserAdapter";
import { applyAskSnapshot } from "../src/execution/streaming";
import { waitForQuietInteractionSurface } from "../src/pageRuntime";
import {
  createBrowserIntegrationHarness,
  createIsolatedTempDir,
  listAttachedPages,
  withAttachedBrowser,
  withHarnessUrl,
} from "./browser-adapter.integration.helpers";

let harness: ReturnType<typeof createBrowserIntegrationHarness>;

beforeEach(() => {
  harness = createBrowserIntegrationHarness();
  resetProviderPressureForTest();
});

afterEach(async () => {
  resetProviderPressureForTest();
  await harness.cleanup();
});

describe("browser adapter", () => {
  it("launches a managed browser and opens the fake harness", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it.each([
    ["classic", "classic"],
    ["aria", "aria"],
    ["menu", "menu"],
    ["live", "live"],
  ] as const)(
    "captures the harness capability matrix for the %s variant",
    async (_label, variant) => {
      const server = await harness.startServer();
      const root = await harness.createWorkspace("browser");
      const targetUrl = withHarnessUrl(server.url, {
        variant,
        initialModel: "thinking",
      });
      const adapter = harness.trackAdapter(
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
    },
  );

  it("keeps the effort control absent until Thinking is selected on the modern menu harness", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "menu",
      initialModel: "instant",
    });
    const adapter = harness.trackAdapter(
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
  });

  it("captures only Deep research aliases from the live sidebar nav cluster", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, { variant: "live" });
    const adapter = harness.trackAdapter(
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
  });

  it("bounds the Deep Research start delay jitter to the real user range", () => {
    expect(deepResearchStartDelayMs(0, true)).toBe(4_000);
    expect(deepResearchStartDelayMs(0.5, true)).toBe(7_000);
    expect(deepResearchStartDelayMs(1, true)).toBe(10_000);
  });

  it("disables the Deep Research start delay under vitest by default", () => {
    expect(process.env.VITEST).toBe("true");
    expect(deepResearchStartDelayMs()).toBe(0);
  });

  it("bounds short interaction jitters to human-scale ranges", () => {
    expect(interactionJitterMs("chip_clear", 0, true)).toBe(30);
    expect(interactionJitterMs("chip_clear", 1, true)).toBe(80);
    expect(interactionJitterMs("keyboard_submit", 0, true)).toBe(35);
    expect(interactionJitterMs("keyboard_submit", 1, true)).toBe(80);
    expect(interactionJitterMs("picker_trigger", 0, true)).toBe(45);
    expect(interactionJitterMs("picker_trigger", 1, true)).toBe(120);
    expect(interactionJitterMs("choice_select", 0, true)).toBe(55);
    expect(interactionJitterMs("choice_select", 1, true)).toBe(145);
    expect(interactionJitterMs("mode_toggle", 0, true)).toBe(60);
    expect(interactionJitterMs("mode_toggle", 1, true)).toBe(150);
    expect(interactionJitterMs("upload_entry", 0, true)).toBe(70);
    expect(interactionJitterMs("upload_entry", 1, true)).toBe(160);
    expect(interactionJitterMs("button_click", 0, true)).toBe(70);
    expect(interactionJitterMs("button_click", 1, true)).toBe(170);
  });

  it("disables short interaction jitters under vitest by default", () => {
    expect(process.env.VITEST).toBe("true");
    expect(interactionJitterMs("button_click", 1)).toBe(0);
  });

  it("bounds high-risk pacing cooldowns and supports deterministic test pacing", () => {
    const previousDeterministic = process.env.TABLINQ_PACING_DETERMINISTIC;

    try {
      expect(highRiskActionCooldownMs("picker_open", 0, true, 1_000)).toBe(70);
      expect(highRiskActionCooldownMs("picker_open", 1, true, 1_000)).toBe(130);
      expect(highRiskActionCooldownMs("final_submit", 0, true, 1_000)).toBe(
        100,
      );
      expect(highRiskActionCooldownMs("retry_click", 1, true, 1_000)).toBe(320);

      process.env.TABLINQ_PACING_DETERMINISTIC = "true";
      expect(highRiskActionCooldownMs("final_submit", 0, true, 1_000)).toBe(
        140,
      );
    } finally {
      if (previousDeterministic === undefined) {
        delete process.env.TABLINQ_PACING_DETERMINISTIC;
      } else {
        process.env.TABLINQ_PACING_DETERMINISTIC = previousDeterministic;
      }
    }
  });

  it("widens cooldowns and adaptive polling under provider pressure and decays afterward", () => {
    const baselineCooldown = highRiskActionCooldownMs(
      "final_submit",
      0.5,
      true,
      1_000,
    );
    const baselinePoll = adaptivePollIntervalMs("generation", 6_000, 1_000);

    expect(providerPressureCooldownMs(1_000)).toBe(0);
    noteProviderPressure("generation_failed", 1_000);
    expect(providerPressureCooldownMs(1_001)).toBeGreaterThan(0);
    expect(
      highRiskActionCooldownMs("final_submit", 0.5, true, 1_001),
    ).toBeGreaterThan(baselineCooldown);
    expect(adaptivePollIntervalMs("generation", 6_000, 1_001)).toBeGreaterThan(
      baselinePoll,
    );

    noteProviderPressureFromText("429 queue full, try again later", 1_500);
    expect(providerPressureCooldownMs(1_501)).toBeGreaterThan(
      providerPressureCooldownMs(21_001),
    );
    expect(providerPressureCooldownMs(60_000)).toBe(0);
  });

  it("waits for a quiet surface before a high-risk submit interaction", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, { variant: "live" });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );

    await adapter.open({ targetUrl, headless: true });
    const page = adapter.getManagedPageForTest()!;
    await page.evaluate(() => {
      const root = document.querySelector("[data-testid='composer-root']");
      if (!(root instanceof HTMLElement)) {
        throw new Error("composer root missing");
      }

      let tick = 0;
      const interval = window.setInterval(() => {
        root.dataset.quietProbe = String(tick);
        tick += 1;
        if (tick >= 4) {
          window.clearInterval(interval);
        }
      }, 60);
    });

    const startedAt = Date.now();
    const settled = await waitForQuietInteractionSurface(
      page,
      "final_submit",
      undefined,
      2_000,
    );

    expect(settled).toBe(true);
    expect(Date.now() - startedAt).toBeGreaterThanOrEqual(300);

    await adapter.shutdown();
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

  it("detects reloads on the managed page", async () => {
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

    await adapter
      .getManagedPageForTest()!
      .reload({ waitUntil: "domcontentloaded" });
    const snapshot = await adapter.health();

    expect(snapshot.reload_count).toBeGreaterThan(0);

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

  it("counts reloads in attach mode", async () => {
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
    await adapter
      .getManagedPageForTest()!
      .reload({ waitUntil: "domcontentloaded" });

    const snapshot = await adapter.health();
    expect(snapshot.reload_count).toBeGreaterThan(0);
    expect(snapshot.browser_owner).toBe("external");

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

  it("serializes complex markdown table cells without breaking table columns", async () => {
    const runtimeSource = await readFile(
      path.join(
        process.cwd(),
        "packages/browser-adapter/src/pageAdapter.runtime.js",
      ),
      "utf8",
    );
    const expected = [
      "| Key | Value | Notes |",
      "| --- | --- | --- |",
      "| Simple | Alpha | Beta |",
      "| Pipe | A \\| B | stays one cell |",
      "| Code | `left \\| right` | inline code stays inline |",
      "| Link | [Docs \\| API](https://example.test/docs\\|api) | link stays parseable |",
      "| Multiline | first<br>second | still one cell |",
      "| Ragged | fewer cells |",
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
    article.setAttribute("data-message-id", "msg-complex-table");
    article.innerHTML = `
      <div class="markdown">
        <table>
          <thead>
            <tr><th>Key</th><th>Value</th><th>Notes</th></tr>
          </thead>
          <tbody>
            <tr><td>Simple</td><td>Alpha</td><td>Beta</td></tr>
            <tr><td>Pipe</td><td>A | B</td><td>stays one cell</td></tr>
            <tr><td>Code</td><td><code>left | right</code></td><td>inline code stays inline</td></tr>
            <tr><td>Link</td><td><a href="https://example.test/docs|api">Docs | API</a></td><td>link stays parseable</td></tr>
            <tr><td>Multiline</td><td>first<br>second</td><td>still one cell</td></tr>
            <tr><td>Ragged</td><td>fewer cells</td></tr>
          </tbody>
        </table>
      </div>
    `;
    window.document.body.append(article);
    window.document.body.dataset.conversationId = "conv-complex-table";
    window.document.body.dataset.lastMessageId = "msg-complex-table";
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

  it("serializes mermaid preview blocks into explicit text instead of a bare mermaid label", async () => {
    const runtimeSource = await readFile(
      path.join(
        process.cwd(),
        "packages/browser-adapter/src/pageAdapter.runtime.js",
      ),
      "utf8",
    );
    const dom = new JSDOM("<!doctype html><html><body></body></html>", {
      url: "https://chatgpt.com/c/conv-mermaid-preview",
      runScripts: "outside-only",
    });
    const { window } = dom;
    const article = window.document.createElement("article");
    article.setAttribute("data-slot", "assistant-message");
    article.hidden = false;
    article.setAttribute("data-message-author-role", "assistant");
    article.setAttribute("data-message-id", "msg-mermaid-preview");
    article.innerHTML = `
      <div class="markdown">
        <pre class="overflow-visible! px-0!">
          <div class="sticky z-2 select-none top-(--sticky-padding-top)">
            <div class="flex w-full items-center justify-between py-1.5 ps-4 pe-1.5 font-sans md:ps-5">
              <div>Mermaid</div>
              <div class="flex flex-row items-center gap-0.5 justify-self-end">
                <button aria-label="Download PNG" type="button"></button>
                <button aria-label="Code" type="button" aria-pressed="false"></button>
                <button aria-label="Preview" type="button" aria-pressed="true"></button>
              </div>
            </div>
          </div>
          <div class="flex w-full flex-col items-center">
            <img src="data:image/svg+xml;utf8,${encodeURIComponent(
              '<svg xmlns="http://www.w3.org/2000/svg"><text>Exercise</text><text>Recovery Loop</text></svg>',
            )}" />
          </div>
        </pre>
      </div>
    `;
    window.document.body.append(article);
    window.document.body.dataset.conversationId = "conv-mermaid-preview";
    window.document.body.dataset.lastMessageId = "msg-mermaid-preview";
    window.document.body.dataset.streamState = "done";

    const snapshot = window.eval(
      `${runtimeSource.replace(
        /^export\s+/gm,
        "",
      )}\ncreateTablinqPageAdapterRuntime().readAskSnapshot();`,
    );

    expect(snapshot.text).toBe(
      "Mermaid diagram preview: Exercise; Recovery Loop.",
    );
    dom.window.close();
  });

  it("ignores transient hidden inline nodes so ask streaming can keep a stable prefix", async () => {
    const runtimeSource = await readFile(
      path.join(
        process.cwd(),
        "packages/browser-adapter/src/pageAdapter.runtime.js",
      ),
      "utf8",
    );
    const dom = new JSDOM("<!doctype html><html><body></body></html>", {
      url: "http://127.0.0.1:3847/?variant=live",
      runScripts: "outside-only",
    });
    const { window } = dom;
    const article = window.document.createElement("article");
    article.setAttribute("data-slot", "assistant-message");
    article.hidden = false;
    article.setAttribute("data-message-author-role", "assistant");
    article.setAttribute("data-message-id", "msg-streaming-rich");
    article.innerHTML = `
      <div class="markdown prose dark:prose-invert w-full wrap-break-word light markdown-new-styling">
        <h1>🧩 Backe<span hidden> </span>nd Programming Languages — Deep Comparison (2025)</h1>
        <p>Below is a </p>
      </div>
    `;
    window.document.body.append(article);
    window.document.body.dataset.conversationId = "conv-streaming-rich";
    window.document.body.dataset.lastMessageId = "msg-streaming-rich";
    window.document.body.dataset.streamState = "running";

    const readAskSnapshot = () =>
      window.eval(
        `${runtimeSource.replace(
          /^export\s+/gm,
          "",
        )}\ncreateTablinqPageAdapterRuntime().readAskSnapshot();`,
      );

    const firstSnapshot = readAskSnapshot();
    expect(firstSnapshot.text).toBe(
      "# 🧩 Backend Programming Languages — Deep Comparison (2025)\n\nBelow is a",
    );

    article.innerHTML = `
      <div class="markdown prose dark:prose-invert w-full wrap-break-word light markdown-new-styling">
        <h1>🧩 Backend Programming Languages — Deep Comparison (2025)</h1>
        <p>Below is a <strong>structured, nested comparison</strong> of the most widely used backend languages.</p>
      </div>
    `;

    const secondSnapshot = readAskSnapshot();
    const chunks: string[] = [];
    const state = {
      emittedText: "",
      nextSeq: 0,
      knownMessageId: null,
      pendingMessageId: null,
      pendingMessageText: "",
      lastSnapshotSignature: null,
    };
    const command = {
      jobId: "job-streaming-rich",
    } as Parameters<typeof applyAskSnapshot>[0];

    const firstProgressed = await applyAskSnapshot(
      command,
      state,
      firstSnapshot,
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );
    const secondProgressed = await applyAskSnapshot(
      command,
      state,
      secondSnapshot,
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );

    expect(firstProgressed).toBe(true);
    expect(secondProgressed).toBe(true);
    expect(chunks).toEqual([]);
    dom.window.close();
  });

  it("reads modern section-based assistant turns and tracks stable turn progress", async () => {
    const runtimeSource = await readFile(
      path.join(
        process.cwd(),
        "packages/browser-adapter/src/pageAdapter.runtime.js",
      ),
      "utf8",
    );
    const dom = new JSDOM(
      "<!doctype html><html><body><main></main></body></html>",
      {
        url: "https://chatgpt.com/c/conv-section-stream",
        runScripts: "outside-only",
      },
    );
    const { window } = dom;
    const main = window.document.querySelector("main");
    if (!(main instanceof window.HTMLElement)) {
      throw new Error("expected main element");
    }

    main.innerHTML = `
      <section data-testid="conversation-turn-1">
        <h4 class="sr-only">You said:</h4>
        <div class="user-message-bubble-color">
          <div>compare martial arts</div>
        </div>
      </section>
      <section data-testid="conversation-turn-2">
        <h4 class="sr-only">ChatGPT said:</h4>
        <div class="markdown">
          <p>Earlier answer.</p>
        </div>
      </section>
    `;
    window.document.body.dataset.streamState = "idle";

    const adapter = window.eval(
      `${runtimeSource.replace(/^export\s+/gm, "")}\ncreateTablinqPageAdapterRuntime();`,
    );
    const baseline = adapter.readAskBaseline();
    expect(baseline.assistantMessageCount).toBe(1);
    expect(baseline.messageId).toBe("conversation-turn-2");
    expect(baseline.text).toBe("Earlier answer.");

    main.insertAdjacentHTML(
      "beforeend",
      `
        <section data-testid="conversation-turn-3">
          <h4 class="sr-only">You said:</h4>
          <div class="user-message-bubble-color">
            <div>give me a shorter comparison</div>
          </div>
        </section>
        <section data-testid="conversation-turn-4">
          <h4 class="sr-only">ChatGPT said:</h4>
          <div class="markdown">
            <p>High-level</p>
          </div>
        </section>
      `,
    );
    window.document.body.dataset.streamState = "running";

    const firstSnapshot = adapter.readAskSnapshot({ baseline });
    expect(firstSnapshot.messageId).toBe("conversation-turn-4");
    expect(firstSnapshot.text).toBe("High-level");
    expect(firstSnapshot.state).toBe("running");

    const chunks: string[] = [];
    const state = {
      emittedText: "",
      nextSeq: 0,
      knownMessageId: null,
      pendingMessageId: null,
      pendingMessageText: "",
      lastSnapshotSignature: null,
    };
    const command = {
      jobId: "job-section-turn-streaming",
    } as Parameters<typeof applyAskSnapshot>[0];

    const firstProgressed = await applyAskSnapshot(
      command,
      state,
      firstSnapshot,
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );
    expect(firstProgressed).toBe(true);

    const latestTurn = main.querySelector(
      "[data-testid='conversation-turn-4'] .markdown",
    );
    if (!(latestTurn instanceof window.HTMLElement)) {
      throw new Error("expected latest assistant markdown");
    }
    latestTurn.innerHTML = "<p>High-level comparison</p>";

    const secondSnapshot = adapter.readAskSnapshot({ baseline });
    const secondProgressed = await applyAskSnapshot(
      command,
      state,
      secondSnapshot,
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );

    expect(secondSnapshot.messageId).toBe("conversation-turn-4");
    expect(secondSnapshot.text).toBe("High-level comparison");
    expect(secondProgressed).toBe(true);
    expect(chunks).toEqual([]);
    dom.window.close();
  });

  it("tracks provisional ask text without emitting chunks until finalization", async () => {
    const command = {
      jobId: "job-provisional-ask-message-id",
    } as Parameters<typeof applyAskSnapshot>[0];
    const state = {
      emittedText: "",
      nextSeq: 0,
      knownMessageId: null,
      pendingMessageId: null,
      pendingMessageText: "",
      lastSnapshotSignature: null,
    };
    const chunks: string[] = [];
    const onChunk = (chunk: { kind: string; text: string }) => {
      if (chunk.kind === "delta") {
        chunks.push(chunk.text);
      }
    };
    const provisionalSnapshot = {
      hasAssistantMessage: true,
      text: "## 🧩 M ovement Modalities Comparison",
      state: "running",
      failureCode: null,
      failureMessage: null,
      retryVisible: false,
      conversationId: "conv-modalities",
      messageId: null,
      thoughtDividerBeforeMessage: false,
      currentUrl: "https://chatgpt.com/c/conv-modalities",
    };
    const identifiedSnapshot = {
      ...provisionalSnapshot,
      text: [
        "## 🧩 Movement Modalities Comparison",
        "",
        "```",
        "Physical Activity",
        "├── 🧘 Stretching",
        "└── 🏋️ Gym Workout",
        "```",
      ].join("\n"),
      messageId: "msg-modalities",
    };
    const advancedSnapshot = {
      ...identifiedSnapshot,
      text: [
        identifiedSnapshot.text,
        "",
        "## 📊 Side-by-Side Attribute Matrix",
        "",
        "| Attribute | Stretching | Gym Workout |",
      ].join("\n"),
    };

    const provisionalProgressed = await applyAskSnapshot(
      command,
      state,
      provisionalSnapshot,
      onChunk,
    );
    const identifiedProgressed = await applyAskSnapshot(
      command,
      state,
      identifiedSnapshot,
      onChunk,
    );
    const advancedProgressed = await applyAskSnapshot(
      command,
      state,
      advancedSnapshot,
      onChunk,
    );

    expect(provisionalProgressed).toBe(true);
    expect(identifiedProgressed).toBe(true);
    expect(advancedProgressed).toBe(true);
    expect(chunks).toEqual([]);
  });

  it("tracks a completed provisional ask snapshot when no message id ever arrives", async () => {
    const command = {
      jobId: "job-provisional-ask-done",
    } as Parameters<typeof applyAskSnapshot>[0];
    const state = {
      emittedText: "",
      nextSeq: 0,
      knownMessageId: null,
      pendingMessageId: null,
      pendingMessageText: "",
      lastSnapshotSignature: null,
    };
    const chunks: string[] = [];
    const onChunk = (chunk: { kind: string; text: string }) => {
      if (chunk.kind === "delta") {
        chunks.push(chunk.text);
      }
    };
    const runningSnapshot = {
      hasAssistantMessage: true,
      text: "Reply",
      state: "running",
      failureCode: null,
      failureMessage: null,
      retryVisible: false,
      conversationId: "conv-provisional-done",
      messageId: null,
      thoughtDividerBeforeMessage: false,
      currentUrl: "https://chatgpt.com/c/conv-provisional-done",
    };
    const doneSnapshot = {
      ...runningSnapshot,
      text: "Reply exactly with ASK_OK",
      state: "done",
    };

    const runningProgressed = await applyAskSnapshot(
      command,
      state,
      runningSnapshot,
      onChunk,
    );
    const doneProgressed = await applyAskSnapshot(
      command,
      state,
      doneSnapshot,
      onChunk,
    );

    expect(runningProgressed).toBe(true);
    expect(doneProgressed).toBe(true);
    expect(chunks).toEqual([]);
    expect(state.emittedText).toBe("");
    expect(state.pendingMessageText).toBe(doneSnapshot.text);
    expect(state.knownMessageId).toBeNull();
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
    const root = await createIsolatedTempDir("browser-detect");
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
    const root = await createIsolatedTempDir("artifacts");
    const first = await chooseUniqueArtifactPath(root, "report.txt");
    await writeFile(first, "first", "utf8");
    const second = await chooseUniqueArtifactPath(root, "report.txt");

    expect(path.basename(first)).toBe("report.txt");
    expect(path.basename(second)).toBe("report-2.txt");
  });
});
