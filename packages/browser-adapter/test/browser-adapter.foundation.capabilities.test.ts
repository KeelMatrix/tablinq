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

describe("browser adapter foundation: capability probing", () => {
  it.each([
    ["classic", "classic"],
    ["aria", "aria"],
    ["menu", "menu"],
    ["live", "live"],
  ] as const)(
    "captures the harness capability matrix for the %s variant",
    async (_label, variant) => {
      const server = await harness.startServer();
      const targetUrl = withHarnessUrl(server.url, {
        variant,
        initialModel: "thinking",
      });
      const { adapter, snapshot } = await openTrackedManagedAdapter(
        harness,
        targetUrl,
      );

      expect(snapshot.extension_connected).toBe(true);
      expectHarnessCapabilityMatrix(snapshot);
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
    const { adapter, snapshot } = await openManagedHarnessAdapter(
      harness,
      server.url,
      {
        variant: "menu",
        initialModel: "instant",
      },
    );
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
    const { adapter, snapshot } = await openManagedHarnessAdapter(
      harness,
      server.url,
      { variant: "live" },
    );
    const researchAliases =
      snapshot.capability_report?.aliases.modes.research ?? [];

    expect(snapshot.capability_report?.matrix.research_mode_visible).toBe(true);
    expect(researchAliases).toContain("deep research");
    expect(researchAliases).not.toContain("apps");
    expect(researchAliases).not.toContain("codex");
    expect(researchAliases).not.toContain("gpts");
    expect(researchAliases).not.toContain("projects");
    expect(snapshot.capability_report?.observed_ui.search_mode).toMatchObject({
      status: "observed",
      visible: true,
    });
    expect(snapshot.capability_report?.observed_ui.research_flow).toMatchObject(
      {
        status: "observed",
        entrypoint_visible: true,
        on_research_url: false,
      },
    );

    await adapter.shutdown();
  });

  it("reports unsupported Pro selection as blocked without turning it into a supported preset", async () => {
    const server = await harness.startServer();
    const { adapter, snapshot } = await openManagedHarnessAdapter(
      harness,
      server.url,
      {
        variant: "composer-right",
        initialModel: "pro",
        initialEffort: "extended",
      },
    );

    expect(
      snapshot.capability_report?.observed_ui.selected_model,
    ).toMatchObject({
      status: "blocked",
      unsupported_pro_selected: true,
      selected_model_class: null,
    });
    expect(
      snapshot.capability_report?.observed_ui.selected_model.labels,
    ).toContain("pro extended");
    expect(snapshot.capability_report?.observed_ui.thinking_effort.status).toBe(
      "blocked",
    );

    await adapter.shutdown();
  });
});
