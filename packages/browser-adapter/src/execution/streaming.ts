import type {
  AskExecutionCommand,
  ResearchProgressSnapshot,
  StreamChunkSnapshot,
} from "../contracts";
import type { AskSnapshot, ResearchSnapshot } from "../snapshotState";
import { normalizeOptionalString } from "../snapshotState";
import { conversationIdFromUrl } from "../urls";

import type { BrowserExecutionHost } from "./types";

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
  if (snapshot.messageId) {
    state.knownMessageId = snapshot.messageId;
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
    lastSnapshotSignature: string | null;
  },
  snapshot: AskSnapshot,
  onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
): Promise<boolean> {
  void command;
  void onChunk;
  if (snapshot.messageId) {
    state.knownMessageId = snapshot.messageId;
    state.pendingMessageId = null;
    state.pendingMessageText = "";
  } else if (!state.knownMessageId && snapshot.text.length > 0) {
    state.pendingMessageId = "__tablinq_provisional_message__";
    state.pendingMessageText = snapshot.text;
  }
  return noteAskSnapshotProgress(state, snapshot);
}

export function noteAskSnapshotProgress(
  state: { lastSnapshotSignature: string | null },
  snapshot: AskSnapshot,
): boolean {
  const signature = askSnapshotProgressSignature(snapshot);
  if (state.lastSnapshotSignature === signature) {
    return false;
  }
  state.lastSnapshotSignature = signature;
  return true;
}

export function askSnapshotProgressSignature(snapshot: AskSnapshot): string {
  return JSON.stringify([
    snapshot.hasAssistantMessage,
    snapshot.text,
    snapshot.state,
    snapshot.failureCode,
    snapshot.failureMessage,
    snapshot.retryVisible,
    snapshot.hasTransientWidgets ?? false,
    snapshot.conversationId,
    snapshot.messageId,
    snapshot.thoughtDividerBeforeMessage ?? false,
    snapshot.thoughtArtifactSignature ?? "",
    snapshot.currentUrl,
  ]);
}

export async function applyResearchSnapshot(
  state: {
    emittedText: string;
    nextSeq: number;
    lastStatus: string;
    lastMessage: string;
    announcedStart: boolean;
    lastSnapshotSignature: string | null;
  },
  snapshot: ResearchSnapshot,
  onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
): Promise<boolean> {
  const progressed = noteResearchSnapshotProgress(state, snapshot);
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
  if (snapshot.status === "running" && state.lastStatus === "starting") {
    await onUpdate({
      status: "waiting_for_sources",
      message: "gathering sources",
      conversation_id: snapshot.conversationId,
      message_id: snapshot.messageId,
      current_url: snapshot.currentUrl,
    });
    state.lastStatus = "waiting_for_sources";
    state.lastMessage = "";
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
  return progressed;
}

export function noteResearchSnapshotProgress(
  state: { lastSnapshotSignature: string | null },
  snapshot: ResearchSnapshot,
): boolean {
  const signature = researchSnapshotProgressSignature(snapshot);
  if (state.lastSnapshotSignature === signature) {
    return false;
  }
  state.lastSnapshotSignature = signature;
  return true;
}

export function researchSnapshotProgressSignature(
  snapshot: ResearchSnapshot,
): string {
  return JSON.stringify([
    snapshot.status,
    snapshot.statusMessage,
    snapshot.reportText,
    snapshot.conversationId,
    snapshot.messageId,
    snapshot.currentUrl,
  ]);
}
