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

describe("browser adapter foundation: pacing and retry primitives", () => {
  it("bounds the Deep Research start delay jitter to the real user range", () => {
    resetSessionRhythmForTest();
    expect(deepResearchStartDelayMs(0, true, 1_000)).toBe(4_000);
    resetSessionRhythmForTest();
    expect(deepResearchStartDelayMs(0.5, true, 1_000)).toBe(7_000);
    resetSessionRhythmForTest();
    expect(deepResearchStartDelayMs(1, true, 1_000)).toBe(10_000);
  });

  it("bounds short interaction jitters to human-scale ranges", () => {
    const jitter = (
      kind: Parameters<typeof interactionJitterMs>[0],
      randomValue: number,
    ) => {
      resetSessionRhythmForTest();
      return interactionJitterMs(kind, randomValue, true, 1_000);
    };

    expect(jitter("chip_clear", 0)).toBe(30);
    expect(jitter("chip_clear", 1)).toBe(80);
    expect(jitter("keyboard_submit", 0)).toBe(35);
    expect(jitter("keyboard_submit", 1)).toBe(80);
    expect(jitter("picker_trigger", 0)).toBe(45);
    expect(jitter("picker_trigger", 1)).toBe(120);
    expect(jitter("choice_select", 0)).toBe(55);
    expect(jitter("choice_select", 1)).toBe(145);
    expect(jitter("mode_toggle", 0)).toBe(60);
    expect(jitter("mode_toggle", 1)).toBe(150);
    expect(jitter("upload_entry", 0)).toBe(70);
    expect(jitter("upload_entry", 1)).toBe(160);
    expect(jitter("button_click", 0)).toBe(70);
    expect(jitter("button_click", 1)).toBe(170);
  });

  it("disables short interaction jitters under vitest by default", () => {
    expect(process.env.VITEST).toBe("true");
    expect(interactionJitterMs("button_click", 1)).toBe(0);
  });

  it("bounds profile-based high-risk pacing and supports deterministic seeds", () => {
    const previousDeterministic = process.env.TABLINQ_PACING_DETERMINISTIC;
    const cooldown = (
      action: Parameters<typeof highRiskActionCooldownMs>[0],
      randomValue: number | string,
    ) => {
      resetSessionRhythmForTest();
      return highRiskActionCooldownMs(action, randomValue, true, 1_000);
    };

    try {
      expect(cooldown("picker_open", 0)).toBe(150);
      expect(cooldown("picker_open", 1)).toBe(350);
      expect(cooldown("option_select", 0)).toBe(250);
      expect(cooldown("option_select", 1)).toBe(700);
      expect(cooldown("mode_toggle", 0)).toBe(300);
      expect(cooldown("mode_toggle", 1)).toBe(900);
      expect(cooldown("upload_entry", 0)).toBe(500);
      expect(cooldown("upload_entry", 1)).toBe(1_200);
      expect(cooldown("final_submit", 0)).toBe(700);
      expect(cooldown("final_submit", 1)).toBe(1_800);
      expect(cooldown("retry_click", 0)).toBe(1_500);
      expect(cooldown("retry_click", 1)).toBe(5_000);

      expect(cooldown("final_submit", "job-abc")).toBe(
        cooldown("final_submit", "job-abc"),
      );
      expect(pacingRandomValueFromSeed("job-abc")).toBeGreaterThanOrEqual(0);
      expect(pacingRandomValueFromSeed("job-abc")).toBeLessThanOrEqual(1);

      process.env.TABLINQ_PACING_DETERMINISTIC = "true";
      expect(cooldown("final_submit", 0)).toBe(1_250);
    } finally {
      if (previousDeterministic === undefined) {
        delete process.env.TABLINQ_PACING_DETERMINISTIC;
      } else {
        process.env.TABLINQ_PACING_DETERMINISTIC = previousDeterministic;
      }
    }
  });

  it("adds a small session rhythm multiplier that decays after idle time", () => {
    resetSessionRhythmForTest();
    const first = highRiskActionCooldownMs("final_submit", 0, true, 1_000);
    const second = highRiskActionCooldownMs("final_submit", 0, true, 1_400);
    const third = highRiskActionCooldownMs("final_submit", 0, true, 1_800);
    const decayed = highRiskActionCooldownMs("final_submit", 0, true, 5_000);

    expect(first).toBe(700);
    expect(second).toBeGreaterThan(first);
    expect(third).toBeGreaterThan(second);
    expect(third).toBeLessThanOrEqual(1_800);
    expect(decayed).toBe(700);
  });

  it("widens cooldowns and adaptive polling under provider pressure and decays afterward", () => {
    resetSessionRhythmForTest();
    const baselineCooldown = highRiskActionCooldownMs(
      "final_submit",
      0.5,
      true,
      1_000,
    );
    resetSessionRhythmForTest();
    const baselinePickerCooldown = highRiskActionCooldownMs(
      "picker_open",
      0.5,
      true,
      1_000,
    );
    const baselinePoll = adaptivePollIntervalMs("generation", 6_000, 1_000);
    resetSessionRhythmForTest();
    const baselineRetryCooldown = highRiskActionCooldownMs(
      "retry_click",
      0.5,
      true,
      1_000,
    );
    resetSessionRhythmForTest();
    const baselineResearchDelay = deepResearchStartDelayMs(0.5, true, 1_000);

    expect(providerPressureCooldownMs(1_000)).toBe(0);
    noteProviderPressure("generation_failed", 1_000);
    expect(providerPressureCooldownMs(1_001)).toBeGreaterThan(0);
    resetSessionRhythmForTest();
    expect(
      highRiskActionCooldownMs("final_submit", 0.5, true, 1_001),
    ).toBeGreaterThan(baselineCooldown);
    resetSessionRhythmForTest();
    expect(highRiskActionCooldownMs("picker_open", 0.5, true, 1_001)).toBe(
      baselinePickerCooldown,
    );
    resetSessionRhythmForTest();
    expect(
      highRiskActionCooldownMs("retry_click", 0.5, true, 1_001),
    ).toBeGreaterThan(baselineRetryCooldown);
    resetSessionRhythmForTest();
    expect(deepResearchStartDelayMs(0.5, true, 1_001)).toBeGreaterThan(
      baselineResearchDelay,
    );
    expect(adaptivePollIntervalMs("generation", 6_000, 1_001)).toBeGreaterThan(
      baselinePoll,
    );

    noteProviderPressureFromText("429 queue full, try again later", 1_500);
    expect(providerPressureCooldownMs(1_501)).toBeGreaterThan(
      providerPressureCooldownMs(21_001),
    );
    expect(providerPressureCooldownMs(60_000)).toBe(0);
  });

  it("keeps non-pressure pacing bounded without excessive timeout consumption", () => {
    resetSessionRhythmForTest();
    expect(highRiskActionCooldownMs("final_submit", 1, true, 1_000)).toBe(
      1_800,
    );
    resetSessionRhythmForTest();
    expect(highRiskActionCooldownMs("retry_click", 1, true, 1_000)).toBe(5_000);
    resetSessionRhythmForTest();
    expect(interactionJitterMs("keyboard_submit", 1, true, 1_000)).toBe(80);
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

  it("reloads once when the composer is unavailable before interaction", async () => {
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
    const page = adapter.getManagedPageForTest()!;
    await page.evaluate(() => {
      for (const element of Array.from(
        document.querySelectorAll(
          [
            "#prompt-textarea",
            "[data-slot='composer']",
            "[data-testid='composer-root']",
            "textarea",
            "[role='textbox']",
            "[contenteditable='true']",
          ].join(", "),
        ),
      )) {
        element.remove();
      }
    });

    await waitForInteractionSurface(page, {
      reloadOnUnavailable: true,
      composerTimeoutMs: 250,
    });

    expect(
      await page
        .locator("#prompt-textarea, textarea, [role='textbox']")
        .count(),
    ).toBeGreaterThan(0);

    await adapter.shutdown();
  });

  it("does not reload when the composer is unavailable during an active provider request", async () => {
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
    const page = adapter.getManagedPageForTest()!;
    await page.evaluate(() => {
      for (const element of Array.from(
        document.querySelectorAll(
          [
            "#prompt-textarea",
            "[data-slot='composer']",
            "[data-testid='composer-root']",
            "textarea",
            "[role='textbox']",
            "[contenteditable='true']",
          ].join(", "),
        ),
      )) {
        element.remove();
      }
      document.body.dataset.streamState = "running";
      document.body.insertAdjacentHTML(
        "beforeend",
        "<button data-testid='stop-button'>Stop</button>",
      );
    });

    await expect(
      waitForInteractionSurface(page, {
        reloadOnUnavailable: true,
        composerTimeoutMs: 250,
      }),
    ).rejects.toThrow();
    expect(await page.locator("[data-testid='composer-root']").count()).toBe(0);
    expect(await page.evaluate(() => document.body.dataset.streamState)).toBe(
      "running",
    );

    await adapter.shutdown();
  });

  it("does not reload when only live DOM busy markers show an active provider request", async () => {
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
    const page = adapter.getManagedPageForTest()!;
    await page.evaluate(() => {
      for (const element of Array.from(
        document.querySelectorAll(
          [
            "#prompt-textarea",
            "[data-slot='composer']",
            "[data-testid='composer-root']",
            "textarea",
            "[role='textbox']",
            "[contenteditable='true']",
          ].join(", "),
        ),
      )) {
        element.remove();
      }
      document.body.dataset.streamState = "idle";
      document.body.insertAdjacentHTML(
        "beforeend",
        "<main><section data-stream-state='running'><button aria-label='Stop generating'>Stop</button></section></main>",
      );
    });

    await expect(
      waitForInteractionSurface(page, {
        reloadOnUnavailable: true,
        composerTimeoutMs: 250,
      }),
    ).rejects.toThrow();
    expect(
      await page.locator("button[aria-label='Stop generating']").count(),
    ).toBe(1);

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
});
