import {
  STALL_DETECTION_MAX_MS,
  STALL_DETECTION_MIN_MS,
} from "./timing";
import { conversationIdFromUrl } from "./urls";

export interface AskSnapshot {
  hasAssistantMessage: boolean;
  text: string;
  state: string;
  conversationId: string | null;
  messageId: string | null;
  currentUrl: string;
}

export interface AskBaseline {
  assistantMessageCount: number;
  conversationId: string | null;
  messageId: string | null;
  text: string;
}

export interface SubmitStateSnapshot {
  composerText: string;
  composerDisabled: boolean;
  submitVisible: boolean;
  submitEnabled: boolean;
  submitBusy: boolean;
  keyboardReady: boolean;
  attachmentsProcessing: boolean;
  stopVisible: boolean;
  assistantMessageCount: number;
  streamState: string;
}

export interface ResearchSnapshot {
  status: "idle" | "waiting_for_sources" | "running" | "done" | "failed";
  statusMessage: string | null;
  reportText: string;
  conversationId: string | null;
  messageId: string | null;
  currentUrl: string;
}

export interface ResearchSubmissionStateSnapshot {
  composerReady: boolean;
  composerText: string;
  composerDisabled: boolean;
  submitVisible: boolean;
  submitEnabled: boolean;
  submitBusy: boolean;
  onDeepResearchLanding: boolean;
  status: "idle" | "waiting_for_sources" | "running" | "done" | "failed";
  statusMessage: string | null;
  reportText: string;
  conversationId: string | null;
  messageId: string | null;
  currentUrl: string;
  submitted: boolean;
}

export interface DeepResearchFrameSnapshot {
  phase: "planning" | "waiting_for_sources" | "running" | "done" | "failed";
  statusMessage: string | null;
  reportText: string;
  frameUrl: string | null;
  startVisible: boolean;
  startEnabled: boolean;
}

export function computeStallTimeoutMs(
  requestedTimeoutMs: number | undefined,
  fallbackTimeoutMs: number,
): number {
  const total = requestedTimeoutMs ?? fallbackTimeoutMs;
  return Math.max(
    STALL_DETECTION_MIN_MS,
    Math.min(STALL_DETECTION_MAX_MS, Math.floor(total / 4)),
  );
}

export function textRelation(
  emittedText: string,
  observedText: string,
): "same" | "ahead" | "behind" | "mismatch" {
  const normalizedEmitted = normalizeComparableAskText(emittedText);
  const normalizedObserved = normalizeComparableAskText(observedText);
  if (normalizedObserved === normalizedEmitted) {
    return "same";
  }
  if (normalizedObserved.startsWith(normalizedEmitted)) {
    return "ahead";
  }
  if (normalizedEmitted.startsWith(normalizedObserved)) {
    return "behind";
  }
  return "mismatch";
}

export function normalizeComparableAskText(text: string): string {
  const trimmed = text.trim();
  return isTransientAskStatusText(trimmed) ? "" : trimmed;
}

export function isTransientAskStatusText(text: string): boolean {
  return /^thought for\b/i.test(text.trim());
}

export function sameAskCompletionSnapshot(
  left: AskSnapshot,
  right: AskSnapshot,
): boolean {
  return (
    left.state === right.state &&
    left.text === right.text &&
    left.conversationId === right.conversationId &&
    left.messageId === right.messageId &&
    left.currentUrl === right.currentUrl
  );
}

export function sameResearchCompletionSnapshot(
  left: ResearchSnapshot,
  right: ResearchSnapshot,
): boolean {
  return (
    left.status === right.status &&
    left.reportText === right.reportText &&
    left.statusMessage === right.statusMessage &&
    left.conversationId === right.conversationId &&
    left.messageId === right.messageId &&
    left.currentUrl === right.currentUrl
  );
}

export function canFinalizeRecoveredAskText(
  emittedText: string,
  finalText: string,
): boolean {
  if (isTransientAskStatusText(emittedText) && finalText.trim().length > 0) {
    return true;
  }
  const relation = textRelation(emittedText, finalText);
  if (relation !== "mismatch") {
    return true;
  }
  return placeholderCompatibleText(emittedText, finalText);
}

export function canFinalizeRecoveredAskSnapshot(
  emittedText: string,
  knownMessageId: string | null,
  snapshot: Pick<AskSnapshot, "messageId" | "text">,
): boolean {
  if (canFinalizeRecoveredAskText(emittedText, snapshot.text)) {
    return true;
  }
  if (
    !knownMessageId ||
    !snapshot.messageId ||
    snapshot.messageId !== knownMessageId
  ) {
    return false;
  }
  return snapshot.text.trim().length > 0;
}

export function markdownCompatibleRecoveredText(
  emittedText: string,
  finalText: string,
): boolean {
  const normalizedEmitted = normalizeRecoveredAskText(emittedText);
  const normalizedFinal = normalizeRecoveredAskText(finalText);
  if (!normalizedEmitted || !normalizedFinal) {
    return false;
  }
  const shorterLength = Math.min(
    normalizedEmitted.length,
    normalizedFinal.length,
  );
  if (shorterLength < 4) {
    return false;
  }
  return textRelation(normalizedEmitted, normalizedFinal) !== "mismatch";
}

export function normalizeRecoveredAskText(text: string): string {
  return text
    .replace(/\r\n/g, "\n")
    .replace(/^```[^\n]*\n?/gm, "")
    .replace(/^```$/gm, "")
    .replace(/^#{1,6}\s+/gm, "")
    .replace(/^>\s?/gm, "")
    .replace(/^(\s*)(?:[-*+]|\d+\.)\s+/gm, "$1")
    .replace(/^\|?(?:\s*:?-+:?\s*\|)+\s*$/gm, "")
    .replace(/`/g, "")
    .replace(/[*_~]/g, "")
    .replace(/^\s*\|\s*/gm, "")
    .replace(/\s*\|\s*/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

export function placeholderCompatibleText(
  left: string,
  right: string,
): boolean {
  const leftChars = Array.from(left);
  const rightChars = Array.from(right);
  const sharedLength = Math.min(leftChars.length, rightChars.length);

  for (let index = 0; index < sharedLength; index += 1) {
    const leftChar = leftChars[index];
    const rightChar = rightChars[index];
    if (leftChar === rightChar) {
      continue;
    }
    if (
      isTransientPlaceholderChar(leftChar) ||
      isTransientPlaceholderChar(rightChar)
    ) {
      continue;
    }
    return false;
  }

  return leftChars
    .slice(sharedLength)
    .concat(rightChars.slice(sharedLength))
    .every((char) => isTransientPlaceholderChar(char));
}

export function isTransientPlaceholderChar(char: string): boolean {
  return (
    char === "_" ||
    char === "…" ||
    char === "·" ||
    char === "•" ||
    char === "█" ||
    char === "▌"
  );
}

export function researchSubmissionStateToSnapshot(
  state: ResearchSubmissionStateSnapshot,
): ResearchSnapshot {
  return normalizeResearchSnapshot({
    status: state.status,
    statusMessage: state.statusMessage,
    reportText: state.reportText,
    conversationId: state.conversationId,
    messageId: state.messageId,
    currentUrl: state.currentUrl,
  });
}

export function normalizePromptText(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

export function scoreDeepResearchFrame(
  snapshot: DeepResearchFrameSnapshot,
): number {
  return (
    (snapshot.startVisible ? 1_000 : 0) +
    (snapshot.phase === "done" ? 300 : 0) +
    (snapshot.phase === "running" ? 200 : 0) +
    (snapshot.phase === "waiting_for_sources" ? 100 : 0) +
    snapshot.reportText.length
  );
}

export function mergeResearchSnapshots(
  domSnapshot: ResearchSnapshot,
  submission: ResearchSubmissionStateSnapshot | null,
  frameSnapshot: DeepResearchFrameSnapshot | null,
): ResearchSnapshot {
  if (!frameSnapshot) {
    return domSnapshot;
  }

  if (frameSnapshot.phase === "planning") {
    return normalizeResearchSnapshot({
      ...domSnapshot,
      status: "waiting_for_sources",
      statusMessage: frameSnapshot.statusMessage ?? domSnapshot.statusMessage,
      reportText: "",
    });
  }

  const reportText =
    frameSnapshot.reportText.length > domSnapshot.reportText.length
      ? frameSnapshot.reportText
      : domSnapshot.reportText;
  let status = domSnapshot.status;
  if (frameSnapshot.phase === "failed") {
    status = "failed";
  } else if (status === "idle") {
    status = frameSnapshot.phase;
  } else if (
    status !== "failed" &&
    status !== "done" &&
    reportText &&
    submission &&
    submission.submitVisible &&
    submission.submitEnabled &&
    !submission.submitBusy &&
    frameSnapshot.phase === "done"
  ) {
    status = "done";
  } else if (status === "waiting_for_sources" && reportText) {
    status = frameSnapshot.phase === "waiting_for_sources" ? status : "running";
  }

  return normalizeResearchSnapshot({
    ...domSnapshot,
    status,
    statusMessage:
      domSnapshot.statusMessage ?? frameSnapshot.statusMessage ?? null,
    reportText,
  });
}

export function normalizeAskSnapshot(snapshot: AskSnapshot): AskSnapshot {
  return {
    ...snapshot,
    conversationId:
      normalizeOptionalString(snapshot.conversationId) ??
      conversationIdFromUrl(snapshot.currentUrl),
    messageId: normalizeOptionalString(snapshot.messageId),
  };
}

export function normalizeAskBaseline(baseline: AskBaseline): AskBaseline {
  return {
    assistantMessageCount: Number.isFinite(baseline.assistantMessageCount)
      ? Math.max(0, Math.floor(baseline.assistantMessageCount))
      : 0,
    conversationId: normalizeOptionalString(baseline.conversationId),
    messageId: normalizeOptionalString(baseline.messageId),
    text: baseline.text ?? "",
  };
}

export function normalizeResearchSnapshot(
  snapshot: ResearchSnapshot,
): ResearchSnapshot {
  return {
    ...snapshot,
    conversationId:
      normalizeOptionalString(snapshot.conversationId) ??
      conversationIdFromUrl(snapshot.currentUrl),
    messageId: normalizeOptionalString(snapshot.messageId),
    statusMessage: normalizeOptionalString(snapshot.statusMessage),
  };
}

export function normalizeResearchSubmissionState(
  state: ResearchSubmissionStateSnapshot,
): ResearchSubmissionStateSnapshot {
  return {
    ...state,
    composerText: state.composerText ?? "",
    conversationId:
      normalizeOptionalString(state.conversationId) ??
      conversationIdFromUrl(state.currentUrl),
    messageId: normalizeOptionalString(state.messageId),
    statusMessage: normalizeOptionalString(state.statusMessage),
    reportText: state.reportText ?? "",
  };
}

export function normalizeDeepResearchFrameSnapshot(
  snapshot: DeepResearchFrameSnapshot,
): DeepResearchFrameSnapshot {
  return {
    ...snapshot,
    statusMessage: normalizeOptionalString(snapshot.statusMessage),
    reportText: snapshot.reportText ?? "",
    frameUrl: normalizeOptionalString(snapshot.frameUrl),
    startVisible: Boolean(snapshot.startVisible),
    startEnabled: Boolean(snapshot.startEnabled),
  };
}

export function normalizeOptionalString(
  value: string | null | undefined,
): string | null {
  const trimmed = value?.trim();
  return trimmed ? trimmed : null;
}
