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

describe("browser adapter ask: mode, model, and effort configuration", () => {
  it("selects Instant from the current ChatGPT model menu", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      initialModel: "thinking",
    });

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
  });

  it("treats an absent model chip as Instant without opening the model menu", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      initialModel: "instant",
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      document
        .querySelectorAll("[aria-checked='true'], [aria-pressed='true']")
        .forEach((element) => {
          element.setAttribute("aria-checked", "false");
          element.setAttribute("aria-pressed", "false");
        });
      document.querySelectorAll("[data-selected='true']").forEach((element) => {
        if (element instanceof HTMLElement) {
          element.dataset.selected = "false";
        }
      });
    });

    await adapter.ask(
      {
        jobId: "job-ask-menu-instant-no-chip",
        prompt: "keep instant without a model chip",
        mode: "search",
        modelClass: "instant",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      model: document.body.dataset.selectedModel,
      modelMenuClicks: document.body.dataset.modelMenuClicks ?? "0",
      modelOptionClicks: document.body.dataset.modelOptionClicks ?? "0",
    }));

    expect(state).toEqual({
      mode: "search",
      model: "instant",
      modelMenuClicks: "0",
      modelOptionClicks: "0",
    });

    await adapter.shutdown();
  });

  it("clears the Thinking chip to select Instant without opening the model menu", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      initialModel: "thinking",
      initialEffort: "standard",
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      const chip = document.querySelector("#thinking-effort-trigger");
      if (!(chip instanceof HTMLElement)) {
        throw new Error("thinking chip missing");
      }
      chip.setAttribute("aria-label", "Thinking, click to remove");
      chip.addEventListener(
        "click",
        (event) => {
          event.stopImmediatePropagation();
          document.body.dataset.selectedModel = "instant";
          document.body.dataset.selectedEffort = "standard";
          (
            window as unknown as { syncSelectedState?: () => void }
          ).syncSelectedState?.();
        },
        { capture: true },
      );
    });

    await adapter.ask(
      {
        jobId: "job-ask-menu-clear-thinking-chip",
        prompt: "clear thinking chip for instant",
        mode: "search",
        modelClass: "instant",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      mode: document.body.dataset.selectedMode,
      model: document.body.dataset.selectedModel,
      modelMenuClicks: document.body.dataset.modelMenuClicks ?? "0",
      modelOptionClicks: document.body.dataset.modelOptionClicks ?? "0",
      effortMenuClicks: document.body.dataset.effortMenuClicks ?? "0",
    }));

    expect(state).toEqual({
      mode: "search",
      model: "instant",
      modelMenuClicks: "0",
      modelOptionClicks: "0",
      effortMenuClicks: "0",
    });

    await adapter.shutdown();
  });

  it("selects Thinking and Standard through the composer-right model menu without reopening it", async () => {
    const server = await harness.startServer();
    const conversationId = "composer-right-thinking-standard";
    const targetUrl = withHarnessUrl(
      new URL(`/c/${conversationId}`, server.url).toString(),
      {
        variant: "composer-right",
        initialModel: "instant",
        initialEffort: "extended",
      },
    );
    const { adapter } = await openManagedHarnessAdapter(harness, targetUrl);

    await adapter.ask(
      {
        jobId: "job-ask-composer-right-thinking-standard",
        prompt: "configure composer right thinking standard",
        conversationId,
        modelClass: "thinking",
        effort: "standard",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
      modelMenuClicks: document.body.dataset.modelMenuClicks ?? "0",
      modelOptionClicks: document.body.dataset.modelOptionClicks ?? "0",
      effortMenuClicks: document.body.dataset.effortMenuClicks ?? "0",
      effortOptionClicks: document.body.dataset.effortOptionClicks ?? "0",
    }));

    expect(state).toEqual({
      model: "thinking",
      effort: "standard",
      modelMenuClicks: "1",
      modelOptionClicks: "0",
      effortMenuClicks: "1",
      effortOptionClicks: "1",
    });

    await adapter.shutdown();
  });

  it("selects Instant through the composer-right model menu", async () => {
    const server = await harness.startServer();
    const conversationId = "composer-right-instant";
    const targetUrl = withHarnessUrl(
      new URL(`/c/${conversationId}`, server.url).toString(),
      {
        variant: "composer-right",
        initialModel: "thinking",
        initialEffort: "standard",
      },
    );
    const { adapter } = await openManagedHarnessAdapter(harness, targetUrl);

    await adapter.ask(
      {
        jobId: "job-ask-composer-right-instant",
        prompt: "configure composer right instant",
        conversationId,
        modelClass: "instant",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      modelMenuClicks: document.body.dataset.modelMenuClicks ?? "0",
      effortMenuClicks: document.body.dataset.effortMenuClicks ?? "0",
    }));

    expect(state).toEqual({
      model: "instant",
      modelMenuClicks: "1",
      effortMenuClicks: "0",
    });

    await adapter.shutdown();
  });

  it("selects Instant from an unlabeled composer model pill with an unknown thinking label", async () => {
    const server = await harness.startServer();
    const conversationId = "composer-right-unknown-thinking-label-instant";
    const targetUrl = withHarnessUrl(
      new URL(`/c/${conversationId}`, server.url).toString(),
      {
        variant: "composer-right",
        initialModel: "thinking",
        initialEffort: "extended",
      },
    );
    const { adapter } = await openManagedHarnessAdapter(harness, targetUrl);

    await adapter.getManagedPageForTest()!.evaluate(() => {
      const trigger = document.querySelector(
        "[data-harness-model-trigger='composer-right']",
      );
      if (!(trigger instanceof HTMLElement)) {
        throw new Error("composer-right model trigger missing");
      }
      trigger.textContent = "Reasoning high";
    });

    await adapter.ask(
      {
        jobId: "job-ask-composer-right-unknown-thinking-label-instant",
        prompt: "configure composer right unknown thinking label instant",
        conversationId,
        modelClass: "instant",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      modelMenuClicks: document.body.dataset.modelMenuClicks ?? "0",
      effortMenuClicks: document.body.dataset.effortMenuClicks ?? "0",
    }));

    expect(state).toEqual({
      model: "instant",
      modelMenuClicks: "1",
      effortMenuClicks: "0",
    });

    await adapter.shutdown();
  });

  it("does not reopen composer-right model or effort menus when Extended is already visible", async () => {
    const server = await harness.startServer();
    const conversationId = "composer-right-thinking-extended";
    const targetUrl = withHarnessUrl(
      new URL(`/c/${conversationId}`, server.url).toString(),
      {
        variant: "composer-right",
        initialModel: "thinking",
        initialEffort: "extended",
      },
    );
    const { adapter } = await openManagedHarnessAdapter(harness, targetUrl);

    await adapter.ask(
      {
        jobId: "job-ask-composer-right-thinking-extended-selected",
        prompt: "keep composer right thinking extended",
        conversationId,
        modelClass: "thinking",
        effort: "extended",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
      modelMenuClicks: document.body.dataset.modelMenuClicks ?? "0",
      modelOptionClicks: document.body.dataset.modelOptionClicks ?? "0",
      effortMenuClicks: document.body.dataset.effortMenuClicks ?? "0",
      effortOptionClicks: document.body.dataset.effortOptionClicks ?? "0",
    }));

    expect(state).toEqual({
      model: "thinking",
      effort: "extended",
      modelMenuClicks: "0",
      modelOptionClicks: "0",
      effortMenuClicks: "0",
      effortOptionClicks: "0",
    });

    await adapter.shutdown();
  });

  it("selects Extended through the current configure modal without reopening the model picker", async () => {
    const server = await harness.startServer();
    const conversationId = "composer-right-configure-extended";
    const targetUrl = withHarnessUrl(
      new URL(`/c/${conversationId}`, server.url).toString(),
      {
        variant: "composer-right",
        initialModel: "thinking",
        initialEffort: "standard",
        effortInConfigureModal: true,
      },
    );
    const { adapter } = await openManagedHarnessAdapter(harness, targetUrl);

    await adapter.ask(
      {
        jobId: "job-ask-composer-right-configure-extended",
        prompt: "configure composer right thinking extended",
        conversationId,
        modelClass: "thinking",
        effort: "extended",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
      modelMenuClicks: document.body.dataset.modelMenuClicks ?? "0",
      modelOptionClicks: document.body.dataset.modelOptionClicks ?? "0",
      configureModalClicks: document.body.dataset.configureModalClicks ?? "0",
      effortComboboxClicks: document.body.dataset.effortComboboxClicks ?? "0",
      effortOptionClicks: document.body.dataset.effortOptionClicks ?? "0",
      modalOpen: document.body.dataset.intelligenceModalOpen,
    }));

    expect(state).toEqual({
      model: "thinking",
      effort: "extended",
      modelMenuClicks: "1",
      modelOptionClicks: "0",
      configureModalClicks: "1",
      effortComboboxClicks: "1",
      effortOptionClicks: "1",
      modalOpen: "false",
    });

    await adapter.shutdown();
  });

  it("selects Standard through the current configure modal without reopening the model picker", async () => {
    const server = await harness.startServer();
    const conversationId = "composer-right-configure-standard";
    const targetUrl = withHarnessUrl(
      new URL(`/c/${conversationId}`, server.url).toString(),
      {
        variant: "composer-right",
        initialModel: "instant",
        initialEffort: "extended",
        effortInConfigureModal: true,
      },
    );
    const { adapter } = await openManagedHarnessAdapter(harness, targetUrl);

    await adapter.ask(
      {
        jobId: "job-ask-composer-right-configure-standard",
        prompt: "configure composer right thinking standard",
        conversationId,
        modelClass: "thinking",
        effort: "standard",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
      modelMenuClicks: document.body.dataset.modelMenuClicks ?? "0",
      modelOptionClicks: document.body.dataset.modelOptionClicks ?? "0",
      configureModalClicks: document.body.dataset.configureModalClicks ?? "0",
      effortComboboxClicks: document.body.dataset.effortComboboxClicks ?? "0",
      effortOptionClicks: document.body.dataset.effortOptionClicks ?? "0",
      modalOpen: document.body.dataset.intelligenceModalOpen,
    }));

    expect(state).toEqual({
      model: "thinking",
      effort: "standard",
      modelMenuClicks: "1",
      modelOptionClicks: "0",
      configureModalClicks: "1",
      effortComboboxClicks: "1",
      effortOptionClicks: "1",
      modalOpen: "false",
    });

    await adapter.shutdown();
  });

  it("treats a visible Thinking effort pill as Standard without opening controls", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "live",
      initialModel: "thinking",
      initialEffort: "standard",
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      document
        .querySelectorAll("[aria-checked='true'], [aria-pressed='true']")
        .forEach((element) => {
          element.setAttribute("aria-checked", "false");
          element.setAttribute("aria-pressed", "false");
        });
      document.querySelectorAll("[data-selected='true']").forEach((element) => {
        if (element instanceof HTMLElement) {
          element.dataset.selected = "false";
        }
      });
    });

    await adapter.ask(
      {
        jobId: "job-ask-live-thinking-standard-visible-pill",
        prompt: "keep visible thinking standard",
        modelClass: "thinking",
        effort: "standard",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
      modelMenuClicks: document.body.dataset.modelMenuClicks ?? "0",
      effortMenuClicks: document.body.dataset.effortMenuClicks ?? "0",
    }));

    expect(state).toEqual({
      model: "thinking",
      effort: "standard",
      modelMenuClicks: "0",
      effortMenuClicks: "0",
    });

    await adapter.shutdown();
  });

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

  it("selects Instant from the classic inline model controls", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      initialModel: "thinking",
    });

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
  });

  it("selects Thinking and Extended from the aria inline controls", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "aria",
      initialModel: "instant",
      initialEffort: "standard",
    });

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
  });

  it("starts a fresh conversation when a new ask requests one", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "live",
    });

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
  });

  it("selects Thinking from the current ChatGPT model menu and then applies Extended effort", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      initialModel: "instant",
    });

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
  });

  it("reports capability missing when the requested model option is absent from the picker", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      initialModel: "thinking",
      missing: ["instant-model"],
    });

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
  });

  it("reports capability missing when thinking effort controls never appear", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      initialModel: "instant",
      missing: ["effort"],
    });

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
  });

  it("runs ask --model instant on the current live harness", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = harness.trackAdapter(
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
  });

  it("fails with a capability-missing error when search mode is unavailable", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("clears blocking composer chips until the model picker returns before a normal ask", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("keeps the selected thinking state while applying effort when the model picker hides", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
        timeoutMs: 12_000,
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
  });
});
