import type {
  AskExecutionCommand,
  ResearchProgressSnapshot,
  StreamChunkSnapshot,
} from "../contracts";
import type { AskSnapshot, ResearchSnapshot } from "../snapshotState";
import {
  markdownCompatibleRecoveredText,
  normalizeOptionalString,
  placeholderCompatibleText,
} from "../snapshotState";
import { conversationIdFromUrl } from "../urls";

import type { BrowserExecutionHost } from "./types";

const PROVISIONAL_PENDING_MESSAGE_ID = "__tablinq_provisional_message__";

export function rememberAskSnapshot(
  host: BrowserExecutionHost,
  state: { knownConversationId: string | null; knownMessageId: string | null },
  snapshot: AskSnapshot,
): void {
  if (snapshot.currentUrl) {
    host.setCurrentTargetUrl(snapshot.currentUrl);
  }
  const conversationId =
    normalizeOptionalString(snapshot.conversationId) ??
    conversationIdFromUrl(snapshot.currentUrl);
  if (conversationId) {
    state.knownConversationId = conversationId;
  }
}

export function rememberResearchSnapshot(
  host: BrowserExecutionHost,
  state: { knownConversationId: string | null; knownMessageId: string | null },
  snapshot: ResearchSnapshot,
): void {
  if (snapshot.currentUrl) {
    host.setCurrentTargetUrl(snapshot.currentUrl);
  }
  const conversationId =
    normalizeOptionalString(snapshot.conversationId) ??
    conversationIdFromUrl(snapshot.currentUrl);
  if (conversationId) {
    state.knownConversationId = conversationId;
  }
  if (snapshot.messageId) {
    state.knownMessageId = snapshot.messageId;
  }
}

export async function applyAskSnapshot(
  command: AskExecutionCommand,
  state: {
    emittedText: string;
    nextSeq: number;
    knownMessageId: string | null;
    pendingMessageId: string | null;
    pendingMessageText: string;
  },
  snapshot: AskSnapshot,
  onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
): Promise<boolean> {
  const messageId = snapshot.messageId;

  if (!messageId) {
    if (!state.knownMessageId) {
      rememberProvisionalAskMessage(state, snapshot.text);
      if (snapshot.state === "done" && snapshot.text.length > 0) {
        return emitWholeAskSnapshot(command, state, snapshot, onChunk);
      }
      return false;
    }
    clearPendingAskMessage(state);
    return emitAskDeltaFromSnapshot(command, state, snapshot, onChunk);
  }

  if (state.knownMessageId === messageId) {
    clearPendingAskMessage(state);
    return emitAskDeltaFromSnapshot(command, state, snapshot, onChunk);
  }

  const provisionalPendingCompatible =
    state.pendingMessageId === PROVISIONAL_PENDING_MESSAGE_ID &&
    provisionalAskTextCompatible(state.pendingMessageText, snapshot.text);

  if (state.pendingMessageId !== messageId) {
    state.pendingMessageId = messageId;
    state.pendingMessageText = snapshot.text;
    if (provisionalPendingCompatible && snapshot.text.length > 0) {
      return commitPendingAskMessage(command, state, snapshot, onChunk);
    }
    if (snapshot.thoughtDividerBeforeMessage && snapshot.text.length > 0) {
      return commitPendingAskMessage(command, state, snapshot, onChunk);
    }
    if (snapshot.state === "done" && snapshot.text.length > 0) {
      return commitPendingAskMessage(command, state, snapshot, onChunk);
    }
    return false;
  }

  const pendingAdvanced =
    snapshot.text.length > state.pendingMessageText.length &&
    snapshot.text.startsWith(state.pendingMessageText);
  state.pendingMessageText =
    snapshot.text.length >= state.pendingMessageText.length
      ? snapshot.text
      : state.pendingMessageText;

  if (pendingAdvanced) {
    return commitPendingAskMessage(command, state, snapshot, onChunk);
  }
  if (
    (snapshot.thoughtDividerBeforeMessage || snapshot.state === "done") &&
    snapshot.text.length > 0
  ) {
    return commitPendingAskMessage(command, state, snapshot, onChunk);
  }
  return false;
}

async function emitAskDeltaFromSnapshot(
  command: AskExecutionCommand,
  state: {
    emittedText: string;
    nextSeq: number;
    knownMessageId: string | null;
  },
  snapshot: AskSnapshot,
  onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
): Promise<boolean> {
  if (
    snapshot.text.length > state.emittedText.length &&
    snapshot.text.startsWith(state.emittedText)
  ) {
    const delta = snapshot.text.slice(state.emittedText.length);
    state.emittedText = snapshot.text;
    if (snapshot.messageId) {
      state.knownMessageId = snapshot.messageId;
    }
    await onChunk({
      job_id: command.jobId,
      conversation_id: snapshot.conversationId,
      message_id: snapshot.messageId,
      seq: state.nextSeq,
      kind: "delta",
      text: delta,
    });
    state.nextSeq += 1;
    return true;
  }
  return false;
}

async function commitPendingAskMessage(
  command: AskExecutionCommand,
  state: {
    emittedText: string;
    nextSeq: number;
    knownMessageId: string | null;
    pendingMessageId: string | null;
    pendingMessageText: string;
  },
  snapshot: AskSnapshot,
  onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
): Promise<boolean> {
  if (!snapshot.messageId || snapshot.text.length === 0) {
    return false;
  }
  state.emittedText = snapshot.text;
  state.knownMessageId = snapshot.messageId;
  clearPendingAskMessage(state);
  await onChunk({
    job_id: command.jobId,
    conversation_id: snapshot.conversationId,
    message_id: snapshot.messageId,
    seq: state.nextSeq,
    kind: "delta",
    text: snapshot.text,
  });
  state.nextSeq += 1;
  return true;
}

function clearPendingAskMessage(state: {
  pendingMessageId: string | null;
  pendingMessageText: string;
}): void {
  state.pendingMessageId = null;
  state.pendingMessageText = "";
}

function rememberProvisionalAskMessage(
  state: {
    pendingMessageId: string | null;
    pendingMessageText: string;
  },
  text: string,
): void {
  state.pendingMessageId = PROVISIONAL_PENDING_MESSAGE_ID;
  state.pendingMessageText =
    text.length >= state.pendingMessageText.length ? text : state.pendingMessageText;
}

function provisionalAskTextCompatible(
  pendingText: string,
  snapshotText: string,
): boolean {
  if (!pendingText || !snapshotText) {
    return false;
  }
  if (
    snapshotText.startsWith(pendingText) ||
    placeholderCompatibleText(pendingText, snapshotText) ||
    markdownCompatibleRecoveredText(pendingText, snapshotText)
  ) {
    return true;
  }

  const compactPending = pendingText.replace(/\s+/g, "");
  const compactSnapshot = snapshotText.replace(/\s+/g, "");
  return (
    compactPending.length >= 4 &&
    compactSnapshot.length >= compactPending.length &&
    compactSnapshot.startsWith(compactPending)
  );
}

async function emitWholeAskSnapshot(
  command: AskExecutionCommand,
  state: {
    emittedText: string;
    nextSeq: number;
    knownMessageId: string | null;
    pendingMessageId: string | null;
    pendingMessageText: string;
  },
  snapshot: AskSnapshot,
  onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
): Promise<boolean> {
  state.emittedText = snapshot.text;
  if (snapshot.messageId) {
    state.knownMessageId = snapshot.messageId;
  }
  clearPendingAskMessage(state);
  await onChunk({
    job_id: command.jobId,
    conversation_id: snapshot.conversationId,
    message_id: snapshot.messageId,
    seq: state.nextSeq,
    kind: "delta",
    text: snapshot.text,
  });
  state.nextSeq += 1;
  return true;
}

export async function applyResearchSnapshot(
  state: {
    emittedText: string;
    nextSeq: number;
    lastStatus: string;
    lastMessage: string;
    announcedStart: boolean;
  },
  snapshot: ResearchSnapshot,
  onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
): Promise<boolean> {
  let progressed = false;
  if (!state.announcedStart) {
    await onUpdate({
      status: "starting",
      message: "starting research",
      conversation_id: snapshot.conversationId,
      message_id: snapshot.messageId,
      current_url: snapshot.currentUrl,
    });
    state.announcedStart = true;
  }
  if (
    snapshot.status !== state.lastStatus ||
    snapshot.statusMessage !== state.lastMessage
  ) {
    state.lastStatus = snapshot.status;
    state.lastMessage = snapshot.statusMessage ?? "";
    if (
      snapshot.status === "waiting_for_sources" ||
      snapshot.status === "running"
    ) {
      await onUpdate({
        status: snapshot.status,
        message: snapshot.statusMessage ?? undefined,
        conversation_id: snapshot.conversationId,
        message_id: snapshot.messageId,
        current_url: snapshot.currentUrl,
      });
    }
  }
  if (
    snapshot.reportText.length > state.emittedText.length &&
    snapshot.reportText.startsWith(state.emittedText)
  ) {
    const delta = snapshot.reportText.slice(state.emittedText.length);
    state.emittedText = snapshot.reportText;
    await onUpdate({
      status: "running",
      message: snapshot.statusMessage ?? undefined,
      conversation_id: snapshot.conversationId,
      message_id: snapshot.messageId,
      current_url: snapshot.currentUrl,
      chunk_kind: "delta",
      chunk_text: delta,
    });
    state.nextSeq += 1;
    progressed = true;
  }
  return progressed;
}
