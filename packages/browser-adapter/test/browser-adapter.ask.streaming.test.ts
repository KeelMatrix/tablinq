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

async function withAskStreamWatchdogEnv<T>(
  values: {
    intervalMs?: number;
    maxMs?: number;
    stickyReloadMs?: number;
  },
  run: () => Promise<T>,
): Promise<T> {
  const previousInterval = process.env.TABLINQ_ASK_STREAM_SNAPSHOT_INTERVAL_MS;
  const previousMax = process.env.TABLINQ_ASK_STREAM_MAX_MS;
  const previousSticky = process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS;
  if (values.intervalMs !== undefined) {
    process.env.TABLINQ_ASK_STREAM_SNAPSHOT_INTERVAL_MS = String(
      values.intervalMs,
    );
  }
  if (values.maxMs !== undefined) {
    process.env.TABLINQ_ASK_STREAM_MAX_MS = String(values.maxMs);
  }
  if (values.stickyReloadMs !== undefined) {
    process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS = String(
      values.stickyReloadMs,
    );
  }
  try {
    return await run();
  } finally {
    if (previousInterval === undefined) {
      delete process.env.TABLINQ_ASK_STREAM_SNAPSHOT_INTERVAL_MS;
    } else {
      process.env.TABLINQ_ASK_STREAM_SNAPSHOT_INTERVAL_MS = previousInterval;
    }
    if (previousMax === undefined) {
      delete process.env.TABLINQ_ASK_STREAM_MAX_MS;
    } else {
      process.env.TABLINQ_ASK_STREAM_MAX_MS = previousMax;
    }
    if (previousSticky === undefined) {
      delete process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS;
    } else {
      process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS = previousSticky;
    }
  }
}

describe("browser adapter ask: streaming and finalization", () => {
  it("continues a real ask stream after the request timeout has elapsed", async () => {
    const server = await harness.startServer();
    const { adapter } = await openTrackedManagedAdapter(harness, server.url);

    try {
      await withAskStreamWatchdogEnv(
        { intervalMs: 150, maxMs: 15000 },
        async () => {
          const prompt =
            "write a long answer that keeps streaming past the original timeout "
              .repeat(6)
              .trim();
          const result = await adapter.ask(
            {
              jobId: "job-ask-stream-after-timeout",
              prompt,
              timeoutMs: 250,
            },
            () => {},
          );

          expect(result.output_text).toBe(`Fake response for: ${prompt}`);
        },
      );
    } finally {
      await adapter.shutdown();
    }
  });

  it("fails when a real ask stream remains unchanged after a watchdog reload", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, { scenario: "timeout" });
    const { adapter } = await openTrackedManagedAdapter(harness, targetUrl);

    try {
      await withAskStreamWatchdogEnv(
        { intervalMs: 150, maxMs: 5000, stickyReloadMs: 5000 },
        async () => {
          await expect(
            adapter.ask(
              {
                jobId: "job-ask-stream-stuck-after-reload",
                prompt: "stall after a real response starts",
                timeoutMs: 5000,
              },
              () => {},
            ),
          ).rejects.toThrow(/timed out while streaming response/i);
        },
      );
      expect((await adapter.health()).reload_count).toBeGreaterThan(0);
    } finally {
      await adapter.shutdown();
    }
  });

  it("returns a final answer when a watchdog reload reveals completion", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      scenario: "timeout-reload-success",
    });
    const { adapter } = await openTrackedManagedAdapter(harness, targetUrl);

    try {
      await withAskStreamWatchdogEnv(
        { intervalMs: 150, maxMs: 5000, stickyReloadMs: 5000 },
        async () => {
          const result = await adapter.ask(
            {
              jobId: "job-ask-stream-watchdog-reload-final",
              prompt: "finish after the watchdog reloads",
              timeoutMs: 5000,
            },
            () => {},
          );

          expect(result.output_text).toBe(
            "Fake response for: finish after the watchdog reloads",
          );
        },
      );
      expect((await adapter.health()).reload_count).toBeGreaterThan(0);
    } finally {
      await adapter.shutdown();
    }
  });

  it("aborts a changing real ask stream at the max streaming guard", async () => {
    const server = await harness.startServer();
    const { adapter } = await openTrackedManagedAdapter(harness, server.url);

    try {
      await withAskStreamWatchdogEnv(
        { intervalMs: 150, maxMs: 800, stickyReloadMs: 5000 },
        async () => {
          await expect(
            adapter.ask(
              {
                jobId: "job-ask-stream-max-guard",
                prompt: "keep writing until the stream guard stops you ".repeat(
                  40,
                ),
                timeoutMs: 250,
              },
              () => {},
            ),
          ).rejects.toThrow(/timed out while streaming response/i);
        },
      );
    } finally {
      await adapter.shutdown();
    }
  });

  it("aborts on the fourth watchdog stall after three advancing reloads", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      scenario: "stream-stall-reload-advance",
    });
    const { adapter } = await openTrackedManagedAdapter(harness, targetUrl);

    try {
      await withAskStreamWatchdogEnv(
        { intervalMs: 150, maxMs: 5000, stickyReloadMs: 5000 },
        async () => {
          await expect(
            adapter.ask(
              {
                jobId: "job-ask-stream-watchdog-fourth-stall",
                prompt: "advance a little after each reload ".repeat(20),
                timeoutMs: 5000,
              },
              () => {},
            ),
          ).rejects.toThrow(/timed out while streaming response/i);
        },
      );
      expect((await adapter.health()).reload_count).toBeGreaterThanOrEqual(3);
    } finally {
      await adapter.shutdown();
    }
  });

  it("keeps thought artifacts on the existing pre-answer timeout path", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "thinking-artifact-messages",
    });
    const { adapter } = await openTrackedManagedAdapter(harness, targetUrl);

    try {
      await withAskStreamWatchdogEnv(
        { intervalMs: 150, maxMs: 5000, stickyReloadMs: 5000 },
        async () => {
          const result = await adapter.ask(
            {
              jobId: "job-ask-thought-artifacts-before-timeout",
              prompt: "do not count thinking artifacts as answer text",
              timeoutMs: 180,
            },
            () => {},
          );

          expect(result.output_text).toBe(
            "Fake response for: do not count thinking artifacts as answer text",
          );
        },
      );
      expect((await adapter.health()).reload_count).toBeGreaterThan(0);
    } finally {
      await adapter.shutdown();
    }
  });

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
