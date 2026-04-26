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
  resetProviderPressureForTest,
  resolveConversationUrl,
  resolveBrowserLaunchConfig,
  uploadAckTimeoutMsForFiles,
  workspaceKeyFromUrl,
} from "../src/browserAdapter";
import { finalizeAsk } from "../src/execution/finalization";
import { applyAskSnapshot } from "../src/execution/streaming";
import {
  createBrowserIntegrationHarness,
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
  openTrackedManagedAdapter,
} from "./helpers/browserFlow";

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
  it("streams ask responses to completion", async () => {
    const server = await harness.startServer();
    const { adapter } = await openTrackedManagedAdapter(harness, server.url);

    const stream = captureDeltaChunks();
    const result = await adapter.ask(
      {
        jobId: "job-ask-success",
        prompt: "hello streaming world",
      },
      stream.onChunk,
    );

    expect(stream.text()).toBe("Fake response for: hello streaming world");
    expectCompletedAskResult(
      result,
      "Fake response for: hello streaming world",
    );

    await adapter.shutdown();
  });

  it("does not replay the previous reply when a follow-up ask reuses the latest conversation implicitly", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      scenario: "followup-stale",
    });
    const { adapter } = await openTrackedManagedAdapter(harness, targetUrl);

    const first = await adapter.ask(
      {
        jobId: "job-ask-followup-implicit-first",
        prompt: "hello gpt",
      },
      () => {},
    );

    const secondStream = captureDeltaChunks();
    const second = await adapter.ask(
      {
        jobId: "job-ask-followup-implicit-second",
        prompt: "what can you do?",
      },
      secondStream.onChunk,
    );

    expect(second.conversation_id).toBe(first.conversation_id);
    expect(second.output_text).toBe("Fake response for: what can you do?");
    expect(secondStream.text()).toBe(second.output_text);
    expect(secondStream.text()).not.toContain(first.output_text);

    await adapter.shutdown();
  });

  it("does not replay the previous reply when a follow-up ask uses the explicit conversation id", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      scenario: "followup-stale",
    });
    const { adapter } = await openTrackedManagedAdapter(harness, targetUrl);

    const first = await adapter.ask(
      {
        jobId: "job-ask-followup-explicit-first",
        prompt: "hello gpt",
      },
      () => {},
    );

    const secondStream = captureDeltaChunks();
    const second = await adapter.ask(
      {
        jobId: "job-ask-followup-explicit-second",
        prompt: "what can you do?",
        conversationId: first.conversation_id,
      },
      secondStream.onChunk,
    );

    expect(second.conversation_id).toBe(first.conversation_id);
    expect(second.output_text).toBe("Fake response for: what can you do?");
    expect(secondStream.text()).toBe(second.output_text);
    expect(secondStream.text()).not.toContain(first.output_text);

    await adapter.shutdown();
  });

  it("streams a multi-paragraph follow-up ask without repeating earlier text or failing reconciliation", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "followup-rich-text",
    });
    const { adapter } = await openTrackedManagedAdapter(harness, targetUrl);

    const first = await adapter.ask(
      {
        jobId: "job-ask-followup-rich-text-first",
        prompt: "hey",
      },
      () => {},
    );

    const secondStream = captureDeltaChunks();
    const second = await adapter.ask(
      {
        jobId: "job-ask-followup-rich-text-second",
        prompt: "what are your capabilities?",
      },
      secondStream.onChunk,
    );

    const expected =
      "I can help with thinking work and doing work.\n\n" +
      "I can explain concepts, write and edit, summarize, brainstorm, tutor, code, debug, analyze data, compare options, plan projects, draft emails/messages, and turn rough notes into polished documents.";

    expect(second.conversation_id).toBe(first.conversation_id);
    expect(second.artifacts).toHaveLength(0);
    expect(second.output_text).toBe(expected);
    expect(secondStream.text()).toBe(expected);
    expect(secondStream.text()).not.toContain(
      `${first.output_text}\n${first.output_text}`,
    );

    await adapter.shutdown();
  });

  it("suppresses thought artifact messages and streams the actual reply", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "thinking-artifact-messages",
    });
    const { adapter } = await openTrackedManagedAdapter(harness, targetUrl);

    const stream = captureDeltaChunks();
    const result = await adapter.ask(
      {
        jobId: "job-ask-thinking-artifact-messages",
        prompt: "reason carefully about this tradeoff",
      },
      stream.onChunk,
    );

    expect(result.output_text).toBe(
      "Fake response for: reason carefully about this tradeoff",
    );
    expect(stream.text()).toBe(result.output_text);
    expect(stream.chunks).toEqual([result.output_text]);
    expect(stream.text()).not.toContain(
      "I’m reconstructing the prior context",
    );
    expect(stream.text()).not.toContain("The comparison is narrowing");

    await adapter.shutdown();
  });

  it("ignores a live-style thought artifact before the answer starts", async () => {
    const { adapter, dom, main } = await createAskRuntimeDom();
    const baseline = adapter.readAskBaseline();
    main.insertAdjacentHTML("beforeend", liveStyleReasoningTurn());
    dom.window.document.body.dataset.streamState = "running";

    const snapshot = adapter.readAskSnapshot({ baseline });

    expect(snapshot.text).toBe("");
    expect(snapshot.text).not.toContain(liveThoughtArtifactText);
    dom.window.close();
  });

  it("reads only the live-style actual answer after a thought artifact", async () => {
    const { adapter, dom, main } = await createAskRuntimeDom();
    const baseline = adapter.readAskBaseline();
    main.insertAdjacentHTML(
      "beforeend",
      liveStyleReasoningTurn(liveActualAnswerText),
    );
    dom.window.document.body.dataset.streamState = "done";

    const snapshot = adapter.readAskSnapshot({ baseline });

    expect(snapshot.messageId).toBe("actual-answer-msg");
    expect(snapshot.text).toBe(liveActualAnswerText);
    expect(snapshot.text).not.toContain(liveThoughtArtifactText);
    expect(snapshot.text).not.toBe(
      `${liveActualAnswerText}\n\n${liveActualAnswerText}`,
    );
    dom.window.close();
  });

  it("tracks live-style answer rewrites without emitting provisional chunks", async () => {
    const { adapter, dom, main } = await createAskRuntimeDom();
    const baseline = adapter.readAskBaseline();
    main.insertAdjacentHTML("beforeend", liveStyleReasoningTurn());
    dom.window.document.body.dataset.streamState = "running";

    const command = { jobId: "job-live-style-reasoning-stream" } as Parameters<
      typeof applyAskSnapshot
    >[0];
    const state = {
      emittedText: "",
      nextSeq: 0,
      knownMessageId: null,
      pendingMessageId: null,
      pendingMessageText: "",
      lastSnapshotSignature: null,
    };
    const chunks: string[] = [];
    const onChunk = (chunk: { kind: string; text?: string }) => {
      if (chunk.kind === "delta") {
        chunks.push(chunk.text ?? "");
      }
    };

    const artifactSnapshot = adapter.readAskSnapshot({ baseline });
    const artifactProgressed = await applyAskSnapshot(
      command,
      state,
      artifactSnapshot,
      onChunk,
    );

    const grow = dom.window.document.querySelector("#live-reasoning-grow");
    if (!(grow instanceof dom.window.HTMLElement)) {
      throw new Error("expected live reasoning container");
    }
    grow.insertAdjacentHTML("beforeend", liveStyleAnswerMessage("A lot."));
    const firstAnswerSnapshot = adapter.readAskSnapshot({ baseline });
    const firstAnswerProgressed = await applyAskSnapshot(
      command,
      state,
      firstAnswerSnapshot,
      onChunk,
    );

    const answerMarkdown = dom.window.document.querySelector(
      "[data-message-id='actual-answer-msg'] .markdown",
    );
    if (!(answerMarkdown instanceof dom.window.HTMLElement)) {
      throw new Error("expected answer markdown");
    }
    answerMarkdown.innerHTML = `<p>${liveActualAnswerText}</p>`;
    const secondAnswerSnapshot = adapter.readAskSnapshot({ baseline });
    const secondAnswerProgressed = await applyAskSnapshot(
      command,
      state,
      secondAnswerSnapshot,
      onChunk,
    );

    expect(artifactProgressed).toBe(true);
    expect(firstAnswerProgressed).toBe(true);
    expect(secondAnswerProgressed).toBe(true);
    expect(chunks).toEqual([]);
    dom.window.close();
  });

  it("finalizes output_text from the cleaned live-style answer snapshot", async () => {
    const { adapter, dom, main } = await createAskRuntimeDom();
    const baseline = adapter.readAskBaseline();
    main.insertAdjacentHTML(
      "beforeend",
      liveStyleReasoningTurn(liveActualAnswerText),
    );
    dom.window.document.body.dataset.streamState = "done";
    const snapshot = adapter.readAskSnapshot({ baseline });
    const emitted: string[] = [];
    const fakeLocator = {
      first() {
        return this;
      },
      async count() {
        return 0;
      },
      async all() {
        return [];
      },
    };
    const fakePage = {
      locator() {
        return fakeLocator;
      },
    };
    const fakeHost = {
      getDownloadsDir() {
        return process.cwd();
      },
      resolveConversationTarget(conversationId: string | null) {
        return `https://chatgpt.com/c/${conversationId ?? "conv-live-reasoning"}`;
      },
      async ensureManagedPage() {},
    };

    const result = await finalizeAsk(
      fakeHost as any,
      { jobId: "job-live-style-finalize" } as any,
      fakePage as any,
      { emittedText: "", nextSeq: 0 },
      snapshot,
      (chunk) => {
        if (chunk.kind === "delta") {
          emitted.push(chunk.text);
        }
      },
    );

    expect(result.output_text).toBe(liveActualAnswerText);
    expect(result.output_text).not.toContain(liveThoughtArtifactText);
    expect(emitted.join("")).toBe(liveActualAnswerText);
    dom.window.close();
  });

  it("refuses asks when the managed page is a signed-out local login screen", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
  });

  it("selects Instant from the current ChatGPT model menu", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "menu",
      initialModel: "thinking",
    });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

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

  it("selects Instant from the classic inline model controls", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, {
      initialModel: "thinking",
    });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

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
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "aria",
      initialModel: "instant",
      initialEffort: "standard",
    });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

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
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
    });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

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
    await adapter.open({ targetUrl, headless: true });

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
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "menu",
      initialModel: "thinking",
      missing: ["instant-model"],
    });
    const adapter = harness.trackAdapter(
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
      /ui drift: could not find a concrete control for model=instant/i,
    );

    await adapter.shutdown();
  });

  it("reports capability missing when thinking effort controls never appear", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "menu",
      initialModel: "instant",
      missing: ["effort"],
    });
    const adapter = harness.trackAdapter(
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

  it("drives the live-like harness through ask, download recovery, and ready-state checks", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = harness.trackAdapter(
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
        modelClass: "thinking",
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
      researchLandingVisited: document.body.dataset.researchLandingVisited,
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
      researchLandingVisited: "true",
      sendHidden: false,
      stopHidden: true,
    });
    expect(result.output_text).toContain("Research report for: configure live");
    expect(result.artifacts).toHaveLength(1);
    expect(artifactBody).toContain("Mode: research");
    expect(artifactBody).toContain("Model: thinking");
    expect(artifactBody).toContain("Effort: extended");
    expect(artifactBody).toContain("Uploads: brief.txt");

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

  it("uploads files and persists generated artifacts", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixtureDir = path.join(root, "fixtures");
    const firstFile = path.join(fixtureDir, "brief.txt");
    const secondFile = path.join(fixtureDir, "notes.csv");
    await mkdir(fixtureDir, { recursive: true });
    await writeFile(firstFile, "brief body", "utf8");
    await writeFile(secondFile, "col\nvalue", "utf8");

    const adapter = harness.trackAdapter(
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
        timeoutMs: 7_000,
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

  it("fails with an upload error when files are requested without an upload control", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = harness.trackAdapter(
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
  });

  it("falls back to keyboard submit when a composer plus button sits next to send", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "submit-upload-trap",
    });
    const adapter = harness.trackAdapter(
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
      uploadChips: document.querySelectorAll("[data-testid='upload-chip']")
        .length,
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
  });

  it("recovers when upload acknowledgment leaves a stale visible send control ahead of the active composer", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "submit-upload-stale-recover",
    });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const result = await adapter.ask(
      {
        jobId: "job-ask-submit-upload-stale-recover",
        prompt: "repeat attached file",
        files: [fixture],
        timeoutMs: 10_000,
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
      staleSubmitClicks: document.body.dataset.staleSubmitClicks,
      keyboardSubmitReady: document.body.dataset.keyboardSubmitReady,
    }));

    expect(result.output_text).toBe("Fake response for: repeat attached file");
    expect(state).toEqual({
      lastSubmissionMethod: "button",
      staleSubmitClicks: "1",
      keyboardSubmitReady: "false",
    });

    await adapter.shutdown();
  });

  it("waits for real submit readiness after upload acknowledgment", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "upload-processing-delay",
    });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const startedAt = Date.now();
    const result = await adapter.ask(
      {
        jobId: "job-ask-upload-processing-delay",
        prompt: "repeat attached file after processing",
        files: [fixture],
        timeoutMs: 10_000,
      },
      () => {},
    );
    const elapsedMs = Date.now() - startedAt;
    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      uploadProcessing: document.body.dataset.uploadProcessing,
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
    }));

    expect(result.output_text).toBe(
      "Fake response for: repeat attached file after processing",
    );
    expect(elapsedMs).toBeGreaterThanOrEqual(1_800);
    expect(state).toEqual({
      uploadProcessing: "false",
      lastSubmissionMethod: "button",
    });

    await adapter.shutdown();
  });

  it("fails clearly when upload acknowledgment never becomes send-ready", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "upload-processing-stuck",
    });
    const adapter = harness.trackAdapter(
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
          jobId: "job-ask-upload-processing-stuck",
          prompt: "repeat attached file",
          files: [fixture],
          timeoutMs: 1_500,
        },
        () => {},
      ),
    ).rejects.toThrow(/attachments are still processing/i);

    await adapter.shutdown();
  });

  it("rejects files larger than 20 MiB before upload", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "oversize.bin");
    await writeFile(fixture, Buffer.alloc(MAX_UPLOAD_FILE_BYTES + 1));
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
      }),
    );
    await adapter.open({ targetUrl: server.url, headless: true });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-oversize-upload",
          prompt: "repeat attached file",
          files: [fixture],
        },
        () => {},
      ),
    ).rejects.toThrow(/file exceeds 20 MiB/i);

    await adapter.shutdown();
  });

  it("fails clearly when submit never becomes ready after upload acknowledgment", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "submit-never-ready",
    });
    const adapter = harness.trackAdapter(
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
          timeoutMs: 1_500,
        },
        () => {},
      ),
    ).rejects.toThrow(/submit never became ready/i);

    await adapter.shutdown();
  });

  it("fails cleanly when the harness never completes streaming", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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
    expect(diagnostics.dom_snapshot).toContain("dom_text");
    expect(diagnostics.aria_snapshot).toContain("role");

    await adapter.shutdown();
  });

  it("streams ask responses to completion in attach mode", async () => {
    const server = await harness.startServer();
    const adapter = await harness.createAdapterForMode("attach", server.url);
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
  });

  it("drives the live-like harness through ask and artifact persistence in attach mode", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      initialModel: "thinking",
      initialEffort: "extended",
    });
    const adapter = await harness.createAdapterForMode("attach", targetUrl);
    const fixtureRoot = await harness.createWorkspace("browser");
    const fixture = path.join(fixtureRoot, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    await adapter.open({ targetUrl, headless: true });

    const result = await adapter.ask(
      {
        jobId: "job-ask-live-attach",
        prompt: "configure attach live",
        mode: "research",
        modelClass: "thinking",
        effort: "extended",
        files: [fixture],
      },
      () => {},
    );
    const artifactBody = await readFile(
      result.artifacts[0]!.storage_path,
      "utf8",
    );

    expect(result.output_text).toContain(
      "Research report for: configure attach live",
    );
    expect(result.artifacts).toHaveLength(1);
    expect(artifactBody).toContain("Model: thinking");
    expect(artifactBody).toContain("Effort: extended");
    expect(artifactBody).toContain("Uploads: brief.txt");
    expect(result.current_url).toContain(`/c/${result.conversation_id}`);
    expect(conversationIdFromUrl(result.current_url)).toBe(
      result.conversation_id,
    );

    await adapter.shutdown();
  });

  it("waits for a stabilized final ask snapshot in attach mode", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "done-early",
    });
    const adapter = await harness.createAdapterForMode("attach", targetUrl);
    const fixtureRoot = await harness.createWorkspace("browser");
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

    expect(result.output_text).toBe(
      "Fake response for: Reply with exactly: ASK_OK",
    );
    expect(chunks.join("")).toBe(result.output_text);

    await adapter.shutdown();
  });

  it("fails fast before composer mutation when attach preflight network checks fail", async () => {
    const server = await harness.startServer();
    const attached = await harness.startAttachableBrowser(server.url);

    const root = await harness.createWorkspace("attach-network-preflight");
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
    await stopFakeChatGptServer(server.process);

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-attach-network-preflight-fail",
          prompt: "should never reach composer",
        },
        () => {},
      ),
    ).rejects.toThrow(/provider\/network-unavailable/i);

    const pageState = await withAttachedBrowser(
      attached.endpoint,
      async (browser) => {
        const page = browser
          .contexts()
          .flatMap((context) => context.pages())
          .find((candidate) => candidate.url().startsWith(server.url));
        if (!page) {
          throw new Error("expected attached target page");
        }
        return page.evaluate(() => ({
          composerValue:
            (
              document.querySelector(
                "#prompt-textarea",
              ) as HTMLTextAreaElement | null
            )?.value ?? "",
          lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
        }));
      },
    );

    expect(pageState.composerValue).toBe("");
    expect(pageState.lastSubmissionMethod).toBe("");
    await adapter.shutdown();
  });

  it("fails with a capability-missing error in attach mode when search mode is unavailable", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, { missing: ["search"] });
    const adapter = await harness.createAdapterForMode("attach", targetUrl);
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
  });

  it("fails cleanly when the harness never completes streaming in attach mode", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, { scenario: "timeout" });
    const adapter = await harness.createAdapterForMode("attach", targetUrl);
    const bundleRoot = await harness.createWorkspace("browser");
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
    expect(diagnostics.dom_snapshot).toContain("dom_text");

    await adapter.shutdown();
  });

  it("does not duplicate submit after a keyboard submit edge is detected", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
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

    await adapter.getManagedPageForTest()!.evaluate(() => {
      const send = document.querySelector("[data-testid='send-button']");
      if (send instanceof HTMLElement) {
        send.remove();
      }
      document.body.dataset.keyboardSubmitReady = "true";
      let keyboardSubmits = 0;
      document
        .querySelector("#prompt-textarea")
        ?.addEventListener("keydown", (event) => {
          if (
            event instanceof KeyboardEvent &&
            event.key === "Enter" &&
            (event.ctrlKey || event.metaKey)
          ) {
            keyboardSubmits += 1;
            document.body.dataset.keyboardSubmitEvents =
              String(keyboardSubmits);
          }
        });
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-keyboard-submit-single-edge",
        prompt: "submit through keyboard once",
        timeoutMs: 7_000,
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      keyboardSubmitEvents: document.body.dataset.keyboardSubmitEvents ?? "0",
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
    }));

    expect(result.output_text).toBe(
      "Fake response for: submit through keyboard once",
    );
    expect(state).toEqual({
      keyboardSubmitEvents: "1",
      lastSubmissionMethod: "keyboard",
    });

    await adapter.shutdown();
  });

  it("waits for the quiet surface before a high-risk submit click", async () => {
    const previousPacing = process.env.TABLINQ_HIGH_RISK_PACING;
    process.env.TABLINQ_HIGH_RISK_PACING = "true";

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

    try {
      await adapter.open({ targetUrl, headless: true });
      await adapter.getManagedPageForTest()!.evaluate(() => {
        const rootElement = document.querySelector(
          "[data-testid='composer-root']",
        );
        const send = document.querySelector("[data-testid='send-button']");
        if (
          !(rootElement instanceof HTMLElement) ||
          !(send instanceof HTMLElement)
        ) {
          throw new Error("composer surface missing");
        }

        let tick = 0;
        const interval = window.setInterval(() => {
          rootElement.dataset.quietSubmitProbe = String(tick);
          tick += 1;
          if (tick >= 12) {
            document.body.dataset.quietMutationEnd = String(performance.now());
            window.clearInterval(interval);
          }
        }, 40);
        send.addEventListener("click", () => {
          document.body.dataset.submitClickAt = String(performance.now());
        });
      });

      await adapter.ask(
        {
          jobId: "job-ask-quiet-surface-submit",
          prompt: "wait before submitting",
          timeoutMs: 8_000,
        },
        () => {},
      );

      const timing = await adapter.getManagedPageForTest()!.evaluate(() => ({
        mutationEnd: Number(document.body.dataset.quietMutationEnd ?? "0"),
        submitClickAt: Number(document.body.dataset.submitClickAt ?? "0"),
      }));

      expect(timing.mutationEnd).toBeGreaterThan(0);
      expect(timing.submitClickAt - timing.mutationEnd).toBeGreaterThanOrEqual(
        150,
      );
    } finally {
      if (previousPacing === undefined) {
        delete process.env.TABLINQ_HIGH_RISK_PACING;
      } else {
        process.env.TABLINQ_HIGH_RISK_PACING = previousPacing;
      }
      await adapter.shutdown();
    }
  });

  it("rejects overlapping interactive actions on the same browser surface", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, { scenario: "timeout" });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const first = adapter.ask(
      {
        jobId: "job-ask-single-flight-first",
        prompt: "hold the browser surface",
        timeoutMs: 600,
      },
      () => {},
    );
    const second = adapter.ask(
      {
        jobId: "job-ask-single-flight-second",
        prompt: "overlap on the same surface",
        timeoutMs: 600,
      },
      () => {},
    );

    await expect(second).rejects.toThrow(/browser surface busy/i);
    await expect(first).rejects.toThrow(/timed out/i);

    await adapter.shutdown();
  });

  it("classifies provider generation failures without clicking Retry", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "provider-generation-failure-twice",
    });
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    let error: any = null;
    try {
      await adapter.ask(
        {
          jobId: "job-ask-provider-generation-failure",
          prompt: "reason carefully about this tradeoff",
        },
        () => {},
      );
    } catch (caught) {
      error = caught;
    }

    expect(error).toBeTruthy();
    expect(error.code).toBe("provider_generation_failed_nonretryable");
    expect(error.retryVisible).toBe(true);
    expect(providerPressureCooldownMs()).toBeGreaterThan(0);
    expect(
      await adapter.getManagedPageForTest()!.evaluate(() => ({
        askFailureCode: document.body.dataset.askFailureCode ?? null,
        retryClicks: document.body.dataset.retryClicks ?? "0",
      })),
    ).toEqual({
      askFailureCode: "provider_generation_failed_nonretryable",
      retryClicks: "0",
    });

    await adapter.shutdown();
  });
});
