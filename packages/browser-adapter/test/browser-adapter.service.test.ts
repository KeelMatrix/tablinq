import type { BrowserAdapter } from "../src/browserAdapter";
import { dispatchAdapterRequest } from "../src/service/dispatch";
import { encodeFrame, streamChunkFrame } from "../src/service/frames";
import {
  fallbackStatusSnapshot,
  normalizeAskCommand,
  normalizeAskRecoveryState,
  normalizeResearchCommand,
  normalizeResearchRecoveryState,
  parseArgs,
} from "../src/service/normalization";
import {
  captureAdapterFrames,
  recoverAskRequestFixture,
  recoverResearchRequestFixture,
  statusSnapshotFixture,
} from "./helpers/serviceFixtures";

describe("browser adapter service helpers", () => {
  it("normalizes CLI args and service execution commands without changing defaults", () => {
    const args = parseArgs([
      "--profile-dir",
      "profile",
      "--downloads-dir",
      "downloads",
      "--default-target-url",
      "https://chatgpt.test",
      "--extension-required",
      "false",
      "--auto-open-target-tab",
      "0",
    ]);

    expect(args).toMatchObject({
      profileDir: "profile",
      downloadsDir: "downloads",
      defaultTargetUrl: "https://chatgpt.test",
      extensionRequired: false,
      autoOpenTargetTab: false,
    });
    expect(
      normalizeAskCommand({ type: "ask", job_id: "job-a", prompt: "hi" }),
    ).toEqual({
      jobId: "job-a",
      prompt: "hi",
      conversationId: undefined,
      newConversation: undefined,
      timeoutMs: undefined,
      submitJitterMs: undefined,
      mode: undefined,
      modelClass: undefined,
      effort: undefined,
      files: [],
    });
    expect(
      normalizeResearchCommand({
        type: "research",
        job_id: "job-r",
        query: "find things",
      }),
    ).toEqual({
      jobId: "job-r",
      query: "find things",
      conversationId: undefined,
      files: [],
      outputDir: undefined,
      timeoutMs: undefined,
      submitJitterMs: undefined,
    });
    expect(fallbackStatusSnapshot(args)).toMatchObject({
      profile_dir: "profile",
      downloads_dir: "downloads",
      target_url: "https://chatgpt.test",
      attach_mode: "managed_only",
      browser_owner: "managed",
      extension_required: false,
    });
  });

  it("maps recover_ask baseline fields with nullish defaults only", () => {
    const request = recoverAskRequestFixture();

    expect(normalizeAskRecoveryState(request)).toEqual({
      emittedText: "",
      nextSeq: 0,
      knownConversationId: null,
      knownMessageId: null,
      askBaseline: {
        assistantMessageCount: 0,
        conversationId: null,
        messageId: "msg-base",
        text: "",
      },
    });
    expect(
      normalizeAskRecoveryState(
        recoverAskRequestFixture({ baseline_assistant_message_count: null }),
      ).askBaseline,
    ).toBeNull();
  });

  it("maps recover_research state fields with the existing defaults", () => {
    expect(normalizeResearchRecoveryState(recoverResearchRequestFixture())).toEqual({
      emittedText: "",
      nextSeq: 0,
    });
    expect(
      normalizeResearchRecoveryState(recoverResearchRequestFixture({
        emitted_text: "partial",
        next_seq: 5,
      })),
    ).toEqual({
      emittedText: "partial",
      nextSeq: 5,
    });
  });

  it("writes stream frames as one JSON line without changing chunk payloads", () => {
    const chunk = {
      job_id: "job-frame",
      conversation_id: "conv-frame",
      message_id: "msg-frame",
      seq: 7,
      kind: "delta" as const,
      text: "hello",
    };
    const frame = streamChunkFrame(chunk);

    expect(frame).toEqual({
      type: "stream_chunk",
      chunk,
    });
    expect(encodeFrame(frame)).toBe(`${JSON.stringify(frame)}\n`);
  });

  it("wraps ask execution callbacks without changing frame shapes or sequence numbers", async () => {
    const frameCapture = captureAdapterFrames();
    const state = statusSnapshotFixture();
    const requestState = {
      state: "running" as const,
      reason: "observed in-flight assistant identity",
      at: "2026-04-27T00:00:00.000Z",
      conversation_id: "conv-ask",
      message_id: "msg-ask",
      current_url: "https://chatgpt.test/c/conv-ask",
      streaming_safe: false,
    };
    const delta = {
      job_id: "job-ask-frame",
      conversation_id: "conv-ask",
      message_id: "msg-ask",
      seq: 3,
      kind: "delta" as const,
      text: "partial",
    };
    const done = {
      ...delta,
      seq: 4,
      kind: "done" as const,
      text: "",
    };
    const result = {
      job_id: "job-ask-frame",
      conversation_id: "conv-ask",
      message_id: "msg-ask",
      output_text: "partial",
      artifacts: [],
      completed_at: "2026-04-27T00:00:01.000Z",
      current_url: "https://chatgpt.test/c/conv-ask",
    };
    const adapter = {
      ask: vi.fn(async (command: any, onChunk: any, onState: any) => {
        expect(command).toEqual({
          jobId: "job-ask-frame",
          prompt: "hello",
          conversationId: undefined,
          newConversation: undefined,
          timeoutMs: undefined,
          submitJitterMs: undefined,
          mode: undefined,
          modelClass: undefined,
          effort: undefined,
          files: [],
        });
        await onState(requestState);
        await onChunk(delta);
        await onChunk(done);
        return result;
      }),
      snapshot: vi.fn(async () => state),
    } as unknown as BrowserAdapter;

    const outcome = await dispatchAdapterRequest(
      adapter,
      { type: "ask", job_id: "job-ask-frame", prompt: "hello" },
      frameCapture.write,
    );

    expect(outcome).toEqual({ shutdown: false });
    expect(frameCapture.frames).toEqual([
      { type: "request_state", update: requestState },
      { type: "stream_chunk", chunk: delta },
      { type: "stream_chunk", chunk: done },
      {
        type: "ask_result",
        ok: true,
        state,
        result,
      },
    ]);
  });
});
