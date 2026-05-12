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

describe("browser adapter ask: live controls configuration", () => {
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

  it("fails safely with diagnostics when drift removes reliable model signals", async () => {
    const server = await harness.startServer();
    const { adapter, root } = await openManagedHarnessAdapter(
      harness,
      server.url,
      {
        variant: "menu",
        driftProfile: "ambiguous",
        initialModel: "instant",
      },
    );

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-ambiguous-drift-thinking",
          prompt: "ambiguous drift should fail",
          modelClass: "thinking",
        },
        () => {},
      ),
    ).rejects.toThrow(/capability missing|ui drift/i);

    const pageState = await adapter.getManagedPageForTest()!.evaluate(() => ({
      selectedModel: document.body.dataset.selectedModel,
      modelMenuClicks: document.body.dataset.modelMenuClicks ?? "0",
      modelOptionClicks: document.body.dataset.modelOptionClicks ?? "0",
    }));
    expect(pageState).toEqual({
      selectedModel: "instant",
      modelMenuClicks: "0",
      modelOptionClicks: "0",
    });

    const diagnostics = await adapter.diagnostics(path.join(root, "bundle"));
    expect(diagnostics.current_url).toContain(server.url);
    expect(diagnostics.capability_matrix).toMatchObject({
      composer_ready: true,
      submit_available: true,
      model_picker_visible: false,
    });
    expect(diagnostics.dom_snapshot).toContain('"dom_text"');
    expect(diagnostics.aria_snapshot).toContain('"role"');
    expect(diagnostics.selector_pack?.pack_id).toBe("chatgpt-default");
    expect(Array.isArray(diagnostics.strategy_telemetry)).toBe(true);
    for (const capability of [
      "composer_ready",
      "submit_available",
      "model_picker_visible",
      "thinking_effort_visible",
      "search_mode_visible",
      "research_mode_visible",
      "upload_control_visible",
      "downloadable_artifacts_observable",
    ]) {
      expect(
        diagnostics.strategy_attempts.some(
          (attempt) => attempt.capability === capability,
        ),
      ).toBe(true);
    }

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
        connectionMode: "managed_only",
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
        connectionMode: "managed_only",
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
        connectionMode: "managed_only",
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
        connectionMode: "managed_only",
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

  it("fails before submit when a visible composer-right state stays ambiguously 5.5-only", async () => {
    const server = await harness.startServer();
    const conversationId = "composer-right-ambiguous-5-5";
    const targetUrl = withHarnessUrl(
      new URL(`/c/${conversationId}`, server.url).toString(),
      {
        variant: "composer-right",
        initialModel: "latest-5-5",
        initialEffort: "extended",
      },
    );
    const { adapter } = await openManagedHarnessAdapter(harness, targetUrl);

    await adapter.getManagedPageForTest()!.evaluate(() => {
      const trigger = document.querySelector(
        "[data-harness-model-trigger='composer-right']",
      );
      const resetVisibleLabel = () => {
        if (trigger instanceof HTMLElement) {
          trigger.textContent = "5.5";
        }
      };
      if (!(trigger instanceof HTMLElement)) {
        throw new Error("composer-right trigger missing");
      }
      resetVisibleLabel();
      document
        .querySelectorAll("#effort-option-standard, #effort-option-extended")
        .forEach((element) => {
          element.addEventListener("click", resetVisibleLabel);
        });
    });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-composer-right-ambiguous-5-5",
          prompt: "do not submit when the visible state is ambiguous 5.5",
          conversationId,
          modelClass: "thinking",
          effort: "extended",
        },
        () => {},
      ),
    ).rejects.toThrow(/capability missing|ui drift/i);

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      model: document.body.dataset.selectedModel,
      effort: document.body.dataset.selectedEffort,
      lastSubmittedModel: document.body.dataset.lastSubmittedModel,
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
      visibleTriggerText:
        document
          .querySelector("[data-harness-model-trigger='composer-right']")
          ?.textContent?.trim() ?? "",
    }));

    expect(state).toEqual({
      model: "thinking",
      effort: "extended",
      lastSubmittedModel: "",
      lastSubmissionMethod: "",
      visibleTriggerText: "5.5",
    });

    await adapter.shutdown();
  });
});
