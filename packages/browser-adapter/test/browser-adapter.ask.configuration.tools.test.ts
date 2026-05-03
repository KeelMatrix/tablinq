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

describe("browser adapter ask: tools configuration", () => {
  it("opens the composer tools menu and selects Web search before submitting", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-menu-search",
        prompt: "configure search mode",
        mode: "search",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      toolsMenuClicks: document.body.dataset.toolsMenuClicks,
      searchToolClicks: document.body.dataset.searchToolClicks,
      searchChipVisible: !document
        .querySelector("#search-chip")
        ?.hasAttribute("hidden"),
    }));

    expect(state).toEqual({
      mode: "search",
      toolsMenuClicks: "1",
      searchToolClicks: "1",
      searchChipVisible: true,
    });
    expect(result.output_text).toContain(
      "Fake response for: configure search mode",
    );

    await adapter.shutdown();
  });

  it("leaves an already-active Search chip selected without reopening tools", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      document.body.dataset.selectedMode = "search";
      (
        window as unknown as { syncSelectedState?: () => void }
      ).syncSelectedState?.();
    });

    await adapter.ask(
      {
        jobId: "job-ask-menu-search-active",
        prompt: "keep search active",
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

  it("clears an active Search chip before a normal ask with matching model controls", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      initialModel: "thinking",
      initialEffort: "standard",
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      document.body.dataset.selectedMode = "search";
      (
        window as unknown as { syncSelectedState?: () => void }
      ).syncSelectedState?.();
    });

    await adapter.ask(
      {
        jobId: "job-ask-menu-normal-clears-search",
        prompt: "clear stale search before normal ask",
        mode: "normal",
        modelClass: "thinking",
        effort: "standard",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
      modelMenuClicks: document.body.dataset.modelMenuClicks ?? "0",
      effortMenuClicks: document.body.dataset.effortMenuClicks ?? "0",
      toolsMenuClicks: document.body.dataset.toolsMenuClicks ?? "0",
      searchToolClicks: document.body.dataset.searchToolClicks ?? "0",
      searchChipVisible: !document
        .querySelector("#search-chip")
        ?.hasAttribute("hidden"),
    }));

    expect(state).toEqual({
      mode: "normal",
      model: "thinking",
      effort: "standard",
      modelMenuClicks: "0",
      effortMenuClicks: "0",
      toolsMenuClicks: "0",
      searchToolClicks: "0",
      searchChipVisible: false,
    });

    await adapter.shutdown();
  });

  it("clears an active Search chip before a normal ask without other configuration", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      document.body.dataset.selectedMode = "search";
      (
        window as unknown as { syncSelectedState?: () => void }
      ).syncSelectedState?.();
    });

    await adapter.ask(
      {
        jobId: "job-ask-menu-normal-only-clears-search",
        prompt: "normal ask after stale search",
        mode: "normal",
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
      mode: "normal",
      toolsMenuClicks: "0",
      searchToolClicks: "0",
      searchChipVisible: false,
    });

    await adapter.shutdown();
  });

  it("clears irrelevant tool chips before a normal ask without changing model chips", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      initialModel: "thinking",
      initialEffort: "standard",
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      const composer = document.querySelector("[data-testid='composer-root']");
      if (!(composer instanceof HTMLElement)) {
        throw new Error("composer setup missing");
      }
      for (const label of [
        "Deep research",
        "Web search",
        "Canvas",
        "Thinking",
      ]) {
        const chip = document.createElement("button");
        chip.type = "button";
        chip.textContent = label;
        chip.className = "__composer-pill";
        chip.setAttribute("aria-label", `${label}, click to remove`);
        chip.addEventListener("click", () => {
          chip.remove();
        });
        composer.appendChild(chip);
      }
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-menu-normal-clears-irrelevant-chips",
        prompt: "normal ask with irrelevant chips",
        mode: "normal",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      remainingLabels: Array.from(
        document.querySelectorAll(
          "[data-testid='composer-root'] button[aria-label*='remove']",
        ),
      ).map((element) => element.textContent),
      mode: document.body.dataset.selectedMode,
      model: document.body.dataset.selectedModel,
      toolsMenuClicks: document.body.dataset.toolsMenuClicks ?? "0",
      searchToolClicks: document.body.dataset.searchToolClicks ?? "0",
    }));

    expect(result.output_text).toContain(
      "Fake response for: normal ask with irrelevant chips",
    );
    expect(state).toEqual({
      remainingLabels: ["Thinking"],
      mode: "normal",
      model: "thinking",
      toolsMenuClicks: "0",
      searchToolClicks: "0",
    });

    await adapter.shutdown();
  });

  it("does not reopen model or effort controls when thinking standard search is already selected", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      initialModel: "thinking",
      initialEffort: "standard",
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      document.body.dataset.selectedMode = "search";
      (
        window as unknown as { syncSelectedState?: () => void }
      ).syncSelectedState?.();
    });

    await adapter.ask(
      {
        jobId: "job-ask-menu-search-all-active",
        prompt: "keep selected controls",
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
      modelMenuClicks: document.body.dataset.modelMenuClicks ?? "0",
      modelOptionClicks: document.body.dataset.modelOptionClicks ?? "0",
      effortMenuClicks: document.body.dataset.effortMenuClicks ?? "0",
      effortOptionClicks: document.body.dataset.effortOptionClicks ?? "0",
      toolsMenuClicks: document.body.dataset.toolsMenuClicks ?? "0",
      searchToolClicks: document.body.dataset.searchToolClicks ?? "0",
    }));

    expect(state).toEqual({
      mode: "search",
      model: "thinking",
      effort: "standard",
      modelMenuClicks: "0",
      modelOptionClicks: "0",
      effortMenuClicks: "0",
      effortOptionClicks: "0",
      toolsMenuClicks: "0",
      searchToolClicks: "0",
    });

    await adapter.shutdown();
  });
});
