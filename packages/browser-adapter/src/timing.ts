export const NAVIGATION_TIMEOUT_MS = 20_000;
export const ASK_TIMEOUT_MS = 25_000;
export const RESEARCH_TIMEOUT_MS = 60_000;
export const PROBE_TIMEOUT_MS = 5_000;
export const ATTACH_PROBE_GRACE_MS = 1_500;
export const INTERACTION_TIMEOUT_MS = 7_500;
export const MAX_UPLOAD_FILE_BYTES = 20 * 1024 * 1024;
export const UPLOAD_ACK_TIMEOUT_MIN_MS = 10_000;
export const UPLOAD_ACK_TIMEOUT_MAX_MS = 40_000;
export const ADDITIONAL_UPLOAD_ACK_WEIGHT = 0.9;
export const SUBMIT_READY_AFTER_UPLOAD_TIMEOUT_MS = 60_000;
export const RESEARCH_SUBMISSION_START_TIMEOUT_MS = 7_500;
export const DEEP_RESEARCH_HANDOFF_TIMEOUT_MS = 60_000;
export const DEEP_RESEARCH_START_CONFIRM_TIMEOUT_MS = 15_000;
export const DEEP_RESEARCH_START_DELAY_MIN_MS = 4_000;
export const DEEP_RESEARCH_START_DELAY_JITTER_MS = 6_000;
export const SUBMIT_START_TIMEOUT_MS = 3_000;
export const DOWNLOAD_TIMEOUT_MS = 10_000;
export const SNAPSHOT_POLL_INTERVAL_MS = 75;
export const STABLE_DONE_POLLS = 3;
export const STALL_DETECTION_MIN_MS = 500;
export const STALL_DETECTION_MAX_MS = 2_000;
export const ASK_STALL_RECOVERY_GRACE_MS = 5_000;
export const MAX_RECOVERY_ATTEMPTS = 3;

export type InteractionJitterKind =
  | "chip_clear"
  | "picker_trigger"
  | "choice_select"
  | "mode_toggle"
  | "button_click"
  | "keyboard_submit"
  | "upload_entry";

export function deepResearchStartDelayMs(randomValue = Math.random()): number {
  const jitter = Math.min(1, Math.max(0, randomValue));
  return (
    DEEP_RESEARCH_START_DELAY_MIN_MS +
    Math.round(jitter * DEEP_RESEARCH_START_DELAY_JITTER_MS)
  );
}

export function interactionJitterMs(
  kind: InteractionJitterKind,
  randomValue = Math.random(),
): number {
  const jitter = Math.min(1, Math.max(0, randomValue));
  switch (kind) {
    case "chip_clear":
      return 30 + Math.round(jitter * 50);
    case "keyboard_submit":
      return 35 + Math.round(jitter * 45);
    case "picker_trigger":
      return 45 + Math.round(jitter * 75);
    case "choice_select":
      return 55 + Math.round(jitter * 90);
    case "mode_toggle":
      return 60 + Math.round(jitter * 90);
    case "upload_entry":
      return 70 + Math.round(jitter * 90);
    case "button_click":
      return 70 + Math.round(jitter * 100);
  }
}

export function uploadAckTimeoutMsForFiles(fileSizes: number[]): number {
  if (fileSizes.length === 0) {
    return UPLOAD_ACK_TIMEOUT_MIN_MS;
  }

  const perFileTimeouts = fileSizes
    .map((value) => uploadAckTimeoutMsForFile(value))
    .sort((left, right) => right - left);
  const [largest, ...rest] = perFileTimeouts;
  return Math.round(
    largest +
      rest.reduce(
        (total, value) => total + value * ADDITIONAL_UPLOAD_ACK_WEIGHT,
        0,
      ),
  );
}

function uploadAckTimeoutMsForFile(fileSizeBytes: number): number {
  const normalizedSize = Math.min(
    1,
    Math.max(0, fileSizeBytes) / MAX_UPLOAD_FILE_BYTES,
  );
  return Math.round(
    UPLOAD_ACK_TIMEOUT_MIN_MS +
      (UPLOAD_ACK_TIMEOUT_MAX_MS - UPLOAD_ACK_TIMEOUT_MIN_MS) *
        Math.sqrt(normalizedSize),
  );
}
