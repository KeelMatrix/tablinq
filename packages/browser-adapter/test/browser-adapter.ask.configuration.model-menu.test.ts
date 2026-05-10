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

describe("browser adapter ask: model menu configuration", () => {
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

  it("submits thinking extended when the composer-right pill only says Extended", async () => {
    const server = await harness.startServer();
    const conversationId = "composer-right-effort-only-extended";
    const targetUrl = withHarnessUrl(
      new URL(`/c/${conversationId}`, server.url).toString(),
      {
        variant: "composer-right",
        initialModel: "thinking",
        initialEffort: "standard",
      },
    );
    const { adapter } = await openManagedHarnessAdapter(harness, targetUrl);

    await adapter.getManagedPageForTest()!.evaluate(() => {
      const forceEffortOnlyText = () => {
        const trigger = document.querySelector(
          "[data-harness-model-trigger='composer-right']",
        );
        if (!(trigger instanceof HTMLElement)) {
          return;
        }
        const effortOnlyText =
          document.body.dataset.selectedEffort === "extended"
            ? "Extended"
            : "Standard";
        if (trigger.textContent !== effortOnlyText) {
          trigger.textContent = effortOnlyText;
        }
      };
      const originalSync = (
        window as unknown as { syncSelectedState?: () => void }
      ).syncSelectedState;
      (
        window as unknown as { syncSelectedState?: () => void }
      ).syncSelectedState = () => {
        originalSync?.();
        forceEffortOnlyText();
      };
      new MutationObserver(forceEffortOnlyText).observe(document.body, {
        attributes: true,
        childList: true,
        characterData: true,
        subtree: true,
      });
      (
        window as unknown as { syncSelectedState?: () => void }
      ).syncSelectedState?.();
    });

    await adapter.ask(
      {
        jobId: "job-ask-composer-right-effort-only-extended",
        prompt: "submit composer right effort only extended",
        conversationId,
        modelClass: "thinking",
        effort: "extended",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
      visible: (
        document.querySelector(
          "[data-harness-model-trigger='composer-right']",
        ) as HTMLElement | null
      )?.textContent,
      lastSubmittedModel: document.body.dataset.lastSubmittedModel,
      lastSubmittedEffort: document.body.dataset.lastSubmittedEffort,
    }));

    expect(state).toEqual({
      model: "thinking",
      effort: "extended",
      visible: "Extended",
      lastSubmittedModel: "thinking",
      lastSubmittedEffort: "extended",
    });

    await adapter.shutdown();
  });

  it("does not treat Pro with an effort-only composer-right pill as supported thinking", async () => {
    const server = await harness.startServer();
    const conversationId = "composer-right-pro-effort-only";
    const targetUrl = withHarnessUrl(
      new URL(`/c/${conversationId}`, server.url).toString(),
      {
        variant: "composer-right",
        initialModel: "pro",
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
      trigger.textContent = "Extended";
    });

    await adapter.ask(
      {
        jobId: "job-ask-composer-right-pro-effort-only",
        prompt: "replace pro effort-only with supported thinking",
        conversationId,
        modelClass: "thinking",
        effort: "extended",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
      lastSubmittedModel: document.body.dataset.lastSubmittedModel,
      lastSubmittedEffort: document.body.dataset.lastSubmittedEffort,
    }));

    expect(state).toEqual({
      model: "thinking",
      effort: "extended",
      lastSubmittedModel: "thinking",
      lastSubmittedEffort: "extended",
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

  it("clears a preselected Pro chip before submitting an Instant ask", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "menu",
      initialModel: "pro",
      initialEffort: "standard",
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      const chip = document.querySelector("#thinking-effort-trigger");
      if (!(chip instanceof HTMLElement)) {
        throw new Error("pro chip missing");
      }
      chip.setAttribute("aria-label", "Pro, click to remove");
      chip.addEventListener(
        "click",
        (event) => {
          event.stopImmediatePropagation();
          document.body.dataset.selectedModel = "instant";
          (
            window as unknown as { syncSelectedState?: () => void }
          ).syncSelectedState?.();
        },
        { capture: true },
      );
    });

    await adapter.ask(
      {
        jobId: "job-ask-menu-clear-pro-chip",
        prompt: "clear pro chip for instant",
        modelClass: "instant",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      lastSubmittedModel: document.body.dataset.lastSubmittedModel,
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
      modelMenuClicks: document.body.dataset.modelMenuClicks ?? "0",
      modelOptionClicks: document.body.dataset.modelOptionClicks ?? "0",
    }));

    expect(state).toEqual({
      model: "instant",
      lastSubmittedModel: "instant",
      lastSubmissionMethod: "button",
      modelMenuClicks: "0",
      modelOptionClicks: "0",
    });

    await adapter.shutdown();
  });

  it("replaces a preselected Pro Thinking state before submitting a thinking-standard ask", async () => {
    const server = await harness.startServer();
    const conversationId = "composer-right-pro-thinking-standard";
    const targetUrl = withHarnessUrl(
      new URL(`/c/${conversationId}`, server.url).toString(),
      {
        variant: "composer-right",
        initialModel: "pro",
        initialEffort: "standard",
      },
    );
    const { adapter } = await openManagedHarnessAdapter(harness, targetUrl);

    await adapter.ask(
      {
        jobId: "job-ask-composer-right-pro-thinking-standard",
        prompt: "replace preselected pro thinking with supported thinking",
        conversationId,
        modelClass: "thinking",
        effort: "standard",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
      lastSubmittedModel: document.body.dataset.lastSubmittedModel,
      lastSubmittedEffort: document.body.dataset.lastSubmittedEffort,
    }));

    expect(state).toEqual({
      model: "thinking",
      effort: "standard",
      lastSubmittedModel: "thinking",
      lastSubmittedEffort: "standard",
    });

    await adapter.shutdown();
  });

  it("replaces a preselected Latest 5.5 state before submitting a thinking-extended ask", async () => {
    const server = await harness.startServer();
    const conversationId = "composer-right-latest-5-5-thinking-extended";
    const targetUrl = withHarnessUrl(
      new URL(`/c/${conversationId}`, server.url).toString(),
      {
        variant: "composer-right",
        initialModel: "latest-5-5",
        initialEffort: "extended",
      },
    );
    const { adapter } = await openManagedHarnessAdapter(harness, targetUrl);

    await adapter.ask(
      {
        jobId: "job-ask-composer-right-latest-5-5-thinking-extended",
        prompt: "replace latest 5.5 with supported extended thinking",
        conversationId,
        modelClass: "thinking",
        effort: "extended",
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
      lastSubmittedModel: document.body.dataset.lastSubmittedModel,
      lastSubmittedEffort: document.body.dataset.lastSubmittedEffort,
    }));

    expect(state).toEqual({
      model: "thinking",
      effort: "extended",
      lastSubmittedModel: "thinking",
      lastSubmittedEffort: "extended",
    });

    await adapter.shutdown();
  });
});
