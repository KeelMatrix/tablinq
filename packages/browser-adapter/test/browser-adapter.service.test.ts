import type {
  BrowserAdapter,
  BrowserStatusSnapshot,
} from "../src/browserAdapter";
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
import type { AdapterFrame, RecoverAskRequest } from "../src/service/protocol";

function statusSnapshot(): BrowserStatusSnapshot {
  return {
    running: false,
    browser_kind: "chromium",
    profile_dir: "profile",
    downloads_dir: "downloads",
    target_url: "https://chatgpt.test",
    current_url: null,
    page_title: null,
    page_ready: false,
    authenticated: false,
    reload_count: 0,
    screenshot_path: null,
    browser_path: null,
    extension_connected: false,
    capability_report: null,
    attach_mode: "managed_only",
    browser_owner: "managed",
    cdp_attached: false,
    target_tab_found: false,
    target_tab_id: null,
    target_workspace_key: null,
    debug_endpoint: "http://127.0.0.1:9222",
    auth_source: "unknown",
    extension_required: true,
    last_capability_report_at: null,
  };
}

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
    const request: RecoverAskRequest = {
      type: "recover_ask",
      job_id: "job-recover",
      prompt: "resume",
      emitted_text: null,
      next_seq: null,
      baseline_assistant_message_count: 0,
      baseline_conversation_id: null,
      baseline_message_id: "msg-base",
      baseline_text: null,
    };

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
      normalizeAskRecoveryState({
        ...request,
        baseline_assistant_message_count: null,
      }).askBaseline,
    ).toBeNull();
  });

  it("maps recover_research state fields with the existing defaults", () => {
    expect(
      normalizeResearchRecoveryState({
        type: "recover_research",
        job_id: "job-research-recover",
        query: "resume research",
        emitted_text: null,
        next_seq: null,
      }),
    ).toEqual({
      emittedText: "",
      nextSeq: 0,
    });
    expect(
      normalizeResearchRecoveryState({
        type: "recover_research",
        job_id: "job-research-recover",
        query: "resume research",
        emitted_text: "partial",
        next_seq: 5,
      }),
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
    const frames: AdapterFrame[] = [];
    const state = statusSnapshot();
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
      (frame) => {
        frames.push(frame);
      },
    );

    expect(outcome).toEqual({ shutdown: false });
    expect(frames).toEqual([
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
