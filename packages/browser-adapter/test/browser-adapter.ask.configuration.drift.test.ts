import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import {
  BrowserAdapter,
  chooseUniqueArtifactPath,
  conversationIdFromUrl,
  deepResearchStartDelayMs,
  detectBrowserExecutable,
  interactionJitterMs,
  MAX_UPLOAD_FILE_BYTES,
  providerPressureCooldownMs,
  resolveConversationUrl,
  resolveBrowserLaunchConfig,
  uploadAckTimeoutMsForFiles,
  workspaceKeyFromUrl,
} from "../src/browserAdapter";
import type { StreamChunkSnapshot } from "../src/browserAdapter";
import { finalizeAsk } from "../src/execution/finalization";
import { applyAskSnapshot } from "../src/execution/streaming";
import {
  createIsolatedTempDir,
  listAttachedPages,
  withAttachedBrowser,
  withHarnessUrl,
} from "./browser-adapter.integration.helpers";
import { stopFakeChatGptServer } from "./helpers";
import {
  createAskRuntimeDom,
  liveActualAnswerText,
  liveStyleAnswerMessage,
  liveStyleReasoningTurn,
  liveThoughtArtifactText,
} from "./helpers/askRuntimeDom";
import {
  captureDeltaChunks,
  expectCompletedAskResult,
  openManagedHarnessAdapter,
  openTrackedManagedAdapter,
} from "./helpers/browserFlow";
import { installBrowserAdapterHarness } from "./helpers/browserAdapterHarness";

const harness = installBrowserAdapterHarness();

describe("browser adapter ask: semantic drift configuration", () => {
  it("activates Search under semantic drift without ChatGPT test ids", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      driftProfile: "semantic",
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-semantic-drift-search",
        prompt: "semantic drift search",
        mode: "search",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      searchChipVisible: !document
        .querySelector("#search-chip")
        ?.hasAttribute("hidden"),
      toolsMenuClicks: document.body.dataset.toolsMenuClicks ?? "0",
      searchToolClicks: document.body.dataset.searchToolClicks ?? "0",
    }));

    expect(result.output_text).toContain(
      "Fake response for: semantic drift search",
    );
    expect(state).toEqual({
      mode: "search",
      searchChipVisible: true,
      toolsMenuClicks: "1",
      searchToolClicks: "1",
    });

    await adapter.shutdown();
  });

  it("selects Thinking Standard under semantic drift", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      driftProfile: "semantic",
      initialModel: "instant",
      initialEffort: "extended",
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-semantic-drift-thinking-standard",
        prompt: "semantic drift thinking standard",
        mode: "search",
        modelClass: "thinking",
        effort: "standard",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
    }));

    expect(result.output_text).toContain(
      "Fake response for: semantic drift thinking standard",
    );
    expect(state).toEqual({
      mode: "search",
      model: "thinking",
      effort: "standard",
    });

    await adapter.shutdown();
  });

  it("selects Instant under selectorless drift without clicking decoys", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      driftProfile: "selectorless",
      initialModel: "thinking",
      initialEffort: "standard",
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-selectorless-drift-instant",
        prompt: "selectorless drift instant",
        modelClass: "instant",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      modelMenuClicks: document.body.dataset.modelMenuClicks ?? "0",
      selectorlessStaleSendClicks:
        document.body.dataset.selectorlessStaleSendClicks ?? "0",
    }));

    expect(result.output_text).toContain(
      "Fake response for: selectorless drift instant",
    );
    expect(state).toEqual({
      model: "instant",
      modelMenuClicks: "1",
      selectorlessStaleSendClicks: "0",
    });

    await adapter.shutdown();
  });

  it("clears stale search and tool chips under selectorless drift", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      driftProfile: "selectorless",
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      document.body.dataset.selectedMode = "search";
      (
        window as unknown as { syncSelectedState?: () => void }
      ).syncSelectedState?.();
      const composer = document.querySelector("form");
      if (!(composer instanceof HTMLElement)) {
        throw new Error("selectorless composer missing");
      }
      for (const label of ["Deep research", "Canvas"]) {
        const chip = document.createElement("button");
        chip.type = "button";
        chip.textContent = label;
        chip.setAttribute("aria-label", `${label}, click to remove`);
        chip.addEventListener("click", () => {
          chip.remove();
        });
        composer.appendChild(chip);
      }
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-selectorless-drift-clears-chips",
        prompt: "selectorless drift clear chips",
        mode: "normal",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      remainingToolChips: Array.from(
        document.querySelectorAll("form button[aria-label*='remove' i]"),
      ).map((element) => element.textContent),
      toolsMenuClicks: document.body.dataset.toolsMenuClicks ?? "0",
      searchToolClicks: document.body.dataset.searchToolClicks ?? "0",
      selectorlessStaleSendClicks:
        document.body.dataset.selectorlessStaleSendClicks ?? "0",
    }));

    expect(result.output_text).toContain(
      "Fake response for: selectorless drift clear chips",
    );
    expect(state).toEqual({
      mode: "normal",
      remainingToolChips: [],
      toolsMenuClicks: "0",
      searchToolClicks: "0",
      selectorlessStaleSendClicks: "0",
    });

    await adapter.shutdown();
  });

  it("does not reopen tools when semantic drift Search is already selected", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      driftProfile: "semantic",
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      document.body.dataset.selectedMode = "search";
      (
        window as unknown as { syncSelectedState?: () => void }
      ).syncSelectedState?.();
    });

    await adapter.ask(
      {
        jobId: "job-ask-semantic-drift-search-active",
        prompt: "semantic drift keep search active",
        mode: "search",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      toolsMenuClicks: document.body.dataset.toolsMenuClicks ?? "0",
      searchToolClicks: document.body.dataset.searchToolClicks ?? "0",
      searchChipVisible: !document
        .querySelector("#search-chip")
        ?.hasAttribute("hidden"),
    }));

    expect(state).toEqual({
      mode: "search",
      toolsMenuClicks: "0",
      searchToolClicks: "0",
      searchChipVisible: true,
    });

    await adapter.shutdown();
  });

  it("retries search activation only when Web search remains unchecked", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      initialModel: "thinking",
      initialEffort: "standard",
      searchClicksBeforeActivate: 1,
    });

    await adapter.ask(
      {
        jobId: "job-ask-menu-search-guarded-retry",
        prompt: "guard search retry",
        mode: "search",
        modelClass: "thinking",
        effort: "standard",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
      toolsMenuClicks: document.body.dataset.toolsMenuClicks,
      searchToolClicks: document.body.dataset.searchToolClicks,
      searchChipVisible: !document
        .querySelector("#search-chip")
        ?.hasAttribute("hidden"),
    }));

    expect(state).toEqual({
      mode: "search",
      model: "thinking",
      effort: "standard",
      toolsMenuClicks: "2",
      searchToolClicks: "2",
      searchChipVisible: true,
    });

    await adapter.shutdown();
  });

  it("keeps Thinking Standard when activating Search resets the model", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      initialModel: "instant",
      initialEffort: "extended",
      searchResetsModelOnActivate: true,
    });

    await adapter.ask(
      {
        jobId: "job-ask-menu-search-reset-thinking-standard",
        prompt: "search after reset",
        mode: "search",
        modelClass: "thinking",
        effort: "standard",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
      toolsMenuClicks: document.body.dataset.toolsMenuClicks ?? "0",
      searchToolClicks: document.body.dataset.searchToolClicks ?? "0",
    }));

    expect(state).toEqual({
      mode: "search",
      model: "thinking",
      effort: "standard",
      toolsMenuClicks: "1",
      searchToolClicks: "1",
    });

    await adapter.shutdown();
  });

  it("falls back to semantic runtime when TypeScript model selectors drift", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      initialModel: "instant",
      initialEffort: "extended",
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      const trigger = document.querySelector("#model-switcher-trigger");
      if (!(trigger instanceof HTMLElement)) {
        throw new Error("model trigger missing");
      }
      trigger.removeAttribute("data-testid");
      trigger.setAttribute("aria-label", "Choose engine");
      trigger.textContent = "Engine";
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-semantic-fallback-search-thinking-standard",
        prompt: "semantic fallback search thinking standard",
        mode: "search",
        modelClass: "thinking",
        effort: "standard",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
      searchChipVisible: !document
        .querySelector("#search-chip")
        ?.hasAttribute("hidden"),
    }));

    expect(state).toEqual({
      mode: "search",
      model: "thinking",
      effort: "standard",
      searchChipVisible: true,
    });
    expect(result.output_text).toContain(
      "Fake response for: semantic fallback search thinking standard",
    );

    await adapter.shutdown();
  });
});
