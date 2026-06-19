import type { Page } from "playwright-core";

import type {
  AskExecutionCommand,
  AskResultSnapshot,
  RequestStateSnapshot,
} from "../contracts";
import { safePageUrl } from "../capabilityProbe";
import type { AskBaseline, AskSnapshot } from "../snapshotState";
import { STABLE_DONE_POLLS } from "../timing";
import { conversationIdFromUrl } from "../urls";

import { finalizeAsk, stabilizeAskCompletion } from "./finalization";
import {
  applyAskSnapshot,
  askSnapshotProviderActivitySignature,
  rememberAskSnapshot,
} from "./streaming";
import type {
  AskExecutionOptions,
  AskRuntimeState,
  BrowserExecutionHost,
  ChunkSink,
  StateSink,
} from "./types";

export function createAskRuntimeState(
  command: AskExecutionCommand,
  options: AskExecutionOptions,
  page: Page,
  askBaseline: AskBaseline | null,
): AskRuntimeState {
  return {
    emittedText: options.emittedText,
    nextSeq: options.nextSeq,
    doneStreak: 0,
    knownConversationId:
      options.knownConversationId ??
      command.conversationId ??
      askBaseline?.conversationId ??
      conversationIdFromUrl(safePageUrl(page)) ??
      null,
    knownMessageId: options.knownMessageId ?? null,
    submittedToProvider: options.submitPrompt,
    submittedConversationId:
      options.submitPrompt
        ? options.knownConversationId ??
          command.conversationId ??
          askBaseline?.conversationId ??
          conversationIdFromUrl(safePageUrl(page)) ??
          null
        : null,
    pendingMessageId: null,
    pendingMessageText: "",
    askBaseline: options.askBaseline ?? askBaseline,
    pendingTextReconciliation: false,
    lastProgressAt: Date.now(),
    lastTextChangeAt: Date.now(),
    lastObservedText: "",
    stickyStreamReloaded: false,
    stickyStreamFallbackAt: null,
    recoveryAttempts: 0,
    lastStatusAt: 0,
    lastSnapshotSignature: null,
    lastProviderActivitySignature: null,
    lastProviderActivityAt: null,
    lastIdentityUpdateSignature: null,
    transientInterruptionThoughtArtifactSignature: null,
  };
}

export async function recordAskSnapshotProgress(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  state: AskRuntimeState,
  snapshot: AskSnapshot,
  onChunk: ChunkSink,
  onState: StateSink,
  reason: string,
): Promise<void> {
  rememberAskSnapshot(host, state, snapshot);
  if (state.submittedToProvider && state.submittedConversationId === null) {
    state.submittedConversationId = state.knownConversationId;
  }
  const progressed = await applyAskSnapshot(command, state, snapshot, onChunk);
  const providerActivitySignature =
    askSnapshotProviderActivitySignature(snapshot);
  state.lastProviderActivitySignature = providerActivitySignature;
  await emitAskIdentityUpdate(
    command,
    state,
    snapshot.currentUrl,
    onState,
    reason,
  );
  if (progressed || providerActivitySignature) {
    const now = Date.now();
    state.lastProgressAt = now;
    if (providerActivitySignature) {
      state.lastProviderActivityAt = now;
    }
  }
  if (snapshot.text !== state.lastObservedText) {
    state.lastObservedText = snapshot.text;
    if (snapshot.text.length > 0) {
      state.lastTextChangeAt = Date.now();
      state.stickyStreamFallbackAt = null;
    }
  }
}

export function updateAskDoneStreak(
  state: AskRuntimeState,
  snapshot: AskSnapshot,
): void {
  if (snapshot.state === "done") {
    state.doneStreak += 1;
  } else {
    state.doneStreak = 0;
  }
}

export async function finalizeStableAskIfReady(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  page: Page,
  state: AskRuntimeState,
  snapshot: AskSnapshot,
  onChunk: ChunkSink,
  onState: StateSink,
  timeoutAt: number,
): Promise<{ snapshot: AskSnapshot; result?: AskResultSnapshot }> {
  if (snapshot.state !== "done" || state.doneStreak < STABLE_DONE_POLLS) {
    return { snapshot };
  }
  const stabilized = await stabilizeAskCompletion(
    page,
    state.askBaseline,
    snapshot,
    timeoutAt,
  );
  await recordAskSnapshotProgress(
    host,
    command,
    state,
    stabilized,
    onChunk,
    onState,
    "observed stabilized assistant identity",
  );
  if (stabilized.state === "done") {
    state.pendingTextReconciliation = false;
    return {
      snapshot: stabilized,
      result: await finalizeAsk(
        host,
        command,
        page,
        state,
        stabilized,
        onChunk,
      ),
    };
  }
  state.doneStreak = 0;
  return { snapshot: stabilized };
}

export async function emitAskIdentityUpdate(
  command: AskExecutionCommand,
  state: {
    knownConversationId: string | null;
    knownMessageId: string | null;
    askBaseline: {
      assistantMessageCount: number;
      conversationId: string | null;
      messageId: string | null;
      text: string;
    } | null;
    lastIdentityUpdateSignature: string | null;
  },
  currentUrl: string | null,
  onState: (update: RequestStateSnapshot) => Promise<void> | void,
  reason: string,
): Promise<void> {
  const signature = JSON.stringify([
    state.knownConversationId,
    state.knownMessageId,
    currentUrl,
    state.askBaseline?.conversationId ?? null,
    state.askBaseline?.messageId ?? null,
    state.askBaseline?.assistantMessageCount ?? null,
    state.askBaseline?.text ?? null,
  ]);
  if (state.lastIdentityUpdateSignature === signature) {
    return;
  }
  state.lastIdentityUpdateSignature = signature;
  await onState({
    state: "running",
    reason,
    at: new Date().toISOString(),
    conversation_id:
      state.knownConversationId ?? command.conversationId ?? null,
    message_id: state.knownMessageId,
    current_url: currentUrl,
    streaming_safe: false,
    baseline_conversation_id: state.askBaseline?.conversationId ?? null,
    baseline_message_id: state.askBaseline?.messageId ?? null,
    baseline_assistant_message_count:
      state.askBaseline?.assistantMessageCount ?? null,
    baseline_text: state.askBaseline?.text ?? null,
  });
}
