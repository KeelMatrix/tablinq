import type { Page } from "playwright-core";

import { logRecoveryDecision, safePageUrl } from "../capabilityProbe";
import type {
  AskExecutionCommand,
  AskResultSnapshot,
  RequestStateSnapshot,
  StreamChunkSnapshot,
} from "../contracts";
import { readAskSnapshot, sleep } from "../pageRuntime";
import type { AskSnapshot } from "../snapshotState";
import {
  canFinalizeRecoveredAskSnapshot,
  isTransientAskStatusText,
  markdownCompatibleRecoveredText,
  normalizeOptionalString,
  placeholderCompatibleText,
  textRelation,
} from "../snapshotState";
import { SNAPSHOT_POLL_INTERVAL_MS, STABLE_DONE_POLLS } from "../timing";
import { conversationIdFromUrl } from "../urls";

import { finalizeAsk, stabilizeAskCompletion } from "./finalization";
import { applyAskSnapshot, rememberAskSnapshot } from "./streaming";
import type {
  AskRecoveryResolution,
  AskRuntimeState,
  BrowserExecutionHost,
} from "./types";

export function assessRecoveredAskSnapshot(
  command: AskExecutionCommand,
  state: {
    emittedText: string;
    knownConversationId: string | null;
    knownMessageId: string | null;
  },
  candidate: AskSnapshot,
): AskRecoveryResolution {
  const relation = textRelation(state.emittedText, candidate.text);
  const relationIsUsable = relation === "same" || relation === "ahead";
  const transientOnlyOutput = isTransientAskStatusText(state.emittedText);
  const commandConversationId = normalizeOptionalString(
    state.knownConversationId ?? command.conversationId,
  );
  const commandMessageId = normalizeOptionalString(state.knownMessageId);
  const snapshotConversationId =
    normalizeOptionalString(candidate.conversationId) ??
    conversationIdFromUrl(candidate.currentUrl);
  const snapshotMessageId = normalizeOptionalString(candidate.messageId);
  const normalizedConversationId =
    snapshotConversationId ?? commandConversationId;
  const conversationMatches =
    !commandConversationId ||
    !snapshotConversationId ||
    commandConversationId === snapshotConversationId;
  const messageMatches =
    commandMessageId !== null &&
    snapshotMessageId !== null &&
    commandMessageId === snapshotMessageId;
  const messageMismatch =
    commandMessageId !== null &&
    snapshotMessageId !== null &&
    commandMessageId !== snapshotMessageId;
  const hasAssistantSignal =
    candidate.hasAssistantMessage &&
    (candidate.messageId !== null || candidate.text.length > 0);
  const sameMessageCompatible =
    relationIsUsable ||
    transientOnlyOutput ||
    placeholderCompatibleText(state.emittedText, candidate.text) ||
    markdownCompatibleRecoveredText(state.emittedText, candidate.text);
  const sameMessageDrift =
    conversationMatches &&
    messageMatches &&
    sameMessageCompatible &&
    (candidate.state === "running" || candidate.state === "done") &&
    hasAssistantSignal;
  const successorMessage =
    conversationMatches &&
    messageMismatch &&
    relationIsUsable &&
    (candidate.state === "running" || candidate.state === "done") &&
    hasAssistantSignal;
  const awaitingLiveState =
    conversationMatches &&
    messageMatches &&
    candidate.state === "idle" &&
    hasAssistantSignal &&
    state.emittedText.length === 0 &&
    candidate.text.length === 0;
  const canContinue =
    (conversationMatches &&
      (relationIsUsable ||
        transientOnlyOutput ||
        sameMessageDrift ||
        successorMessage) &&
      (candidate.state === "running" || candidate.state === "done") &&
      hasAssistantSignal) ||
    awaitingLiveState;
  const hardMismatch =
    !conversationMatches ||
    (messageMatches &&
      candidate.state === "done" &&
      hasAssistantSignal &&
      !sameMessageCompatible &&
      !successorMessage);
  logRecoveryDecision("ask", {
    commandConversationId,
    commandMessageId,
    snapshotConversationId,
    snapshotMessageId,
    snapshotCurrentUrl: candidate.currentUrl,
    normalizedConversationId,
    relation,
    conversationMatches,
    messageMatches,
    messageMismatch,
    hasAssistantSignal,
    snapshotState: candidate.state,
    emittedTextLength: state.emittedText.length,
    observedTextLength: candidate.text.length,
    hasAssistantMessage: candidate.hasAssistantMessage,
    transientOnlyOutput,
    sameMessageCompatible,
    sameMessageDrift,
    successorMessage,
    awaitingLiveState,
    hardMismatch,
    canContinue,
  });
  return {
    candidate,
    relation,
    sameMessageDrift,
    hardMismatch,
    canContinue,
  };
}

export async function tryResumeAskFromPage(
  host: BrowserExecutionHost,
  page: Page,
  command: AskExecutionCommand,
  state: AskRuntimeState,
  onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
  onState: (update: RequestStateSnapshot) => Promise<void> | void,
  timeoutAt: number,
): Promise<{ page: Page; result?: AskResultSnapshot } | null> {
  let resolution: AskRecoveryResolution | null = null;
  try {
    resolution = assessRecoveredAskSnapshot(
      command,
      state,
      await readAskSnapshot(page, state.askBaseline),
    );
  } catch {
    return null;
  }
  if (!resolution.canContinue) {
    return null;
  }

  let snapshot = resolution.candidate;
  rememberAskSnapshot(host, state, snapshot);
  if (resolution.relation === "mismatch" && resolution.sameMessageDrift) {
    state.pendingTextReconciliation = true;
  }
  await onState({
    state: "recovered",
    reason:
      snapshot.state === "done"
        ? "recovered the final assistant reply from the live conversation"
        : "recovered the in-flight assistant reply from the live conversation",
    at: new Date().toISOString(),
    conversation_id: snapshot.conversationId,
    message_id: snapshot.messageId,
    current_url: snapshot.currentUrl,
    streaming_safe: snapshot.state === "running",
  });

  const progressed = await applyAskSnapshot(command, state, snapshot, onChunk);
  if (progressed) {
    state.doneStreak = 0;
  }
  if (snapshot.state === "done") {
    snapshot = await stabilizeAskCompletion(
      page,
      state.askBaseline,
      snapshot,
      timeoutAt,
    );
    rememberAskSnapshot(host, state, snapshot);
    const stabilizedProgress = await applyAskSnapshot(
      command,
      state,
      snapshot,
      onChunk,
    );
    if (stabilizedProgress) {
      state.doneStreak = 0;
    }
  }
  if (snapshot.state !== "done") {
    return { page };
  }
  state.doneStreak = STABLE_DONE_POLLS;
  if (
    state.pendingTextReconciliation &&
    !canFinalizeRecoveredAskSnapshot(
      state.emittedText,
      state.knownMessageId,
      snapshot,
    )
  ) {
    return null;
  }
  state.pendingTextReconciliation = false;
  return {
    page,
    result: await finalizeAsk(host, command, page, state, snapshot, onChunk),
  };
}

export async function reconcileAskAfterInterruption(
  host: BrowserExecutionHost,
  page: Page,
  command: AskExecutionCommand,
  state: AskRuntimeState,
  onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
  onState: (update: RequestStateSnapshot) => Promise<void> | void,
  reason: string,
  timeoutAt: number,
): Promise<{ page: Page; result?: AskResultSnapshot }> {
  state.recoveryAttempts += 1;
  const interruptedAt = new Date().toISOString();
  if (Date.now() - state.lastStatusAt > 50) {
    await onState({
      state: "interrupted",
      reason,
      at: interruptedAt,
      conversation_id: state.knownConversationId,
      message_id: state.knownMessageId,
      current_url: safePageUrl(page),
      streaming_safe: false,
    });
    state.lastStatusAt = Date.now();
  }

  const currentPageRecovery = await tryResumeAskFromPage(
    host,
    page,
    command,
    state,
    onChunk,
    onState,
    timeoutAt,
  );
  if (currentPageRecovery) {
    return currentPageRecovery;
  }

  const recoveredPage = await host.openExecutionPage(
    host.resolveAskTarget(
      state.knownConversationId ??
        command.conversationId ??
        state.askBaseline?.conversationId ??
        conversationIdFromUrl(safePageUrl(page)),
    ),
  );
  if (Date.now() >= timeoutAt) {
    throw new Error(
      "ask timed out while reconciling the interrupted request",
    );
  }

  let snapshot: AskSnapshot | null = null;
  while (Date.now() < timeoutAt) {
    const resolution = assessRecoveredAskSnapshot(
      command,
      state,
      await readAskSnapshot(recoveredPage, state.askBaseline),
    );
    if (resolution.canContinue) {
      rememberAskSnapshot(host, state, resolution.candidate);
      if (resolution.relation === "mismatch" && resolution.sameMessageDrift) {
        state.pendingTextReconciliation = true;
      }
      snapshot = resolution.candidate;
      break;
    }
    if (resolution.hardMismatch) {
      throw new Error(
        "interrupted ask could not be reconciled against the live conversation",
      );
    }
    await sleep(SNAPSHOT_POLL_INTERVAL_MS);
  }
  if (!snapshot) {
    throw new Error(
      "interrupted ask could not be reconciled against the live conversation",
    );
  }

  await onState({
    state: "recovered",
    reason:
      snapshot.state === "done"
        ? "recovered the final assistant reply from the live conversation"
        : "recovered the in-flight assistant reply from the live conversation",
    at: new Date().toISOString(),
    conversation_id: snapshot.conversationId,
    message_id: snapshot.messageId,
    current_url: snapshot.currentUrl,
    streaming_safe: snapshot.state === "running",
  });

  const progressed = await applyAskSnapshot(command, state, snapshot, onChunk);
  if (progressed) {
    state.doneStreak = 0;
  }
  if (snapshot.state === "done") {
    snapshot = await stabilizeAskCompletion(
      recoveredPage,
      state.askBaseline,
      snapshot,
      timeoutAt,
    );
    rememberAskSnapshot(host, state, snapshot);
    const stabilizedProgress = await applyAskSnapshot(
      command,
      state,
      snapshot,
      onChunk,
    );
    if (stabilizedProgress) {
      state.doneStreak = 0;
    }
  }
  if (snapshot.state === "done") {
    state.doneStreak = STABLE_DONE_POLLS;
    if (
      state.pendingTextReconciliation &&
      !canFinalizeRecoveredAskSnapshot(
        state.emittedText,
        state.knownMessageId,
        snapshot,
      )
    ) {
      throw new Error(
        "interrupted ask could not be reconciled against the live conversation",
      );
    }
    state.pendingTextReconciliation = false;
    return {
      page: recoveredPage,
      result: await finalizeAsk(
        host,
        command,
        recoveredPage,
        state,
        snapshot,
        onChunk,
      ),
    };
  }
  return { page: recoveredPage };
}
