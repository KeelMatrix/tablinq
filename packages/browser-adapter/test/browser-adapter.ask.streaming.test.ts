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

describe("browser adapter ask: streaming and finalization", () => {
  it("streams ask responses to completion", async () => {
    const server = await harness.startServer();
    const { adapter } = await openTrackedManagedAdapter(harness, server.url);
    const page = adapter.getManagedPageForTest()!;
    await page.setViewportSize({ width: 900, height: 600 });
    await page.evaluate(() => {
      document.body.insertAdjacentHTML(
        "afterbegin",
        "<div style='height: 2400px'>older conversation</div>",
      );
      window.scrollTo(0, 0);
    });

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
    const remainingPx = await page.evaluate(() => {
      const root = document.scrollingElement ?? document.documentElement;
      return root.scrollHeight - root.clientHeight - root.scrollTop;
    });
    expect(remainingPx).toBeLessThanOrEqual(8);

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
    const emitted: StreamChunkSnapshot[] = [];
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
      async evaluate() {
        return false;
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
        emitted.push(chunk);
      },
    );

    expect(result.output_text).toBe(liveActualAnswerText);
    expect(result.output_text).not.toContain(liveThoughtArtifactText);
    expect(emitted).toEqual([
      {
        job_id: "job-live-style-finalize",
        conversation_id: snapshot.conversationId,
        message_id: snapshot.messageId,
        seq: 0,
        kind: "delta",
        text: liveActualAnswerText,
      },
      {
        job_id: "job-live-style-finalize",
        conversation_id: snapshot.conversationId,
        message_id: snapshot.messageId,
        seq: 1,
        kind: "done",
        text: "",
      },
    ]);
    dom.window.close();
  });
});
