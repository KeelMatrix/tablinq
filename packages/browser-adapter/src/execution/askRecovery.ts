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
  isTransientAskStatusText,
  markdownCompatibleRecoveredText,
  normalizeOptionalString,
  placeholderCompatibleText,
  textRelation,
} from "../snapshotState";
import { adaptivePollIntervalMs, STABLE_DONE_POLLS } from "../timing";
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
    askBaseline?: {
      assistantMessageCount: number;
      conversationId: string | null;
      messageId: string | null;
      text: string;
    } | null;
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
  const askBaseline = state.askBaseline ?? null;
  const postBaselineMessage =
    askBaseline !== null &&
    hasAssistantSignal &&
    (snapshotMessageId === null ||
      askBaseline.messageId === null ||
      snapshotMessageId !== askBaseline.messageId ||
      candidate.text !== askBaseline.text);
  const sameMessageIdentity =
    conversationMatches &&
    messageMatches &&
    (candidate.state === "running" || candidate.state === "done") &&
    hasAssistantSignal;
  const successorMessage =
    conversationMatches &&
    messageMismatch &&
    (candidate.state === "running" || candidate.state === "done") &&
    hasAssistantSignal;
  const baselineSuccessorMessage =
    conversationMatches &&
    !commandMessageId &&
    postBaselineMessage &&
    (candidate.state === "running" || candidate.state === "done");
  const identityCompatible =
    sameMessageIdentity || successorMessage || baselineSuccessorMessage;
  const recoveredTextCompatible =
    relationIsUsable ||
    transientOnlyOutput ||
    placeholderCompatibleText(state.emittedText, candidate.text) ||
    markdownCompatibleRecoveredText(state.emittedText, candidate.text);
  const sameMessageCompatible = identityCompatible || recoveredTextCompatible;
  const sameMessageDrift =
    conversationMatches &&
    messageMatches &&
    sameMessageCompatible &&
    (candidate.state === "running" || candidate.state === "done") &&
    hasAssistantSignal;
  const awaitingLiveState =
    conversationMatches &&
    (messageMatches || baselineSuccessorMessage) &&
    candidate.state === "idle" &&
    hasAssistantSignal &&
    state.emittedText.length === 0 &&
    candidate.text.length === 0;
  const canContinue =
    (conversationMatches &&
      (identityCompatible ||
        relationIsUsable ||
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
      !identityCompatible &&
      !recoveredTextCompatible &&
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
    identityCompatible,
    recoveredTextCompatible,
    sameMessageDrift,
    successorMessage,
    baselineSuccessorMessage,
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
  state.pendingTextReconciliation = false;
  return {
    page,
    result: await finalizeAsk(host, command, page, state, snapshot, onChunk),
  };
}

function recoveryConversationIdForSubmittedAsk(
  page: Page,
  command: AskExecutionCommand,
  state: AskRuntimeState,
): string | null {
  return (
    state.knownConversationId ??
    state.submittedConversationId ??
    command.conversationId ??
    state.askBaseline?.conversationId ??
    conversationIdFromUrl(safePageUrl(page)) ??
    null
  );
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

  const recoveryConversationId = recoveryConversationIdForSubmittedAsk(
    page,
    command,
    state,
  );
  if (!recoveryConversationId && state.submittedToProvider) {
    throw new Error(
      "submitted ask cannot be reconciled because no provider conversation id is known",
    );
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
    host.resolveAskTarget(recoveryConversationId),
    false,
    { failOnProviderConversationMismatch: false },
  );
  if (Date.now() >= timeoutAt) {
    throw new Error("ask timed out while reconciling the interrupted request");
  }

  let snapshot: AskSnapshot | null = null;
  const recoveryWaitStartedAt = Date.now();
  while (Date.now() < timeoutAt) {
    const resolution = assessRecoveredAskSnapshot(
      command,
      state,
      await readAskSnapshot(recoveredPage, state.askBaseline),
    );
    if (resolution.canContinue) {
      rememberAskSnapshot(host, state, resolution.candidate);
      snapshot = resolution.candidate;
      break;
    }
    if (resolution.hardMismatch) {
      throw new Error(
        "interrupted ask could not be reconciled against the live conversation",
      );
    }
    await sleep(
      Math.min(
        adaptivePollIntervalMs("recovery", Date.now() - recoveryWaitStartedAt),
        Math.max(1, timeoutAt - Date.now()),
      ),
    );
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
