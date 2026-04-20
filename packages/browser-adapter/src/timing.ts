export const NAVIGATION_TIMEOUT_MS = 20_000;
export const ASK_TIMEOUT_MS = 25_000;
export const RESEARCH_TIMEOUT_MS = 60_000;
export const PROBE_TIMEOUT_MS = 5_000;
export const ATTACH_PROBE_GRACE_MS = 1_500;
export const INTERACTION_TIMEOUT_MS = 7_500;
export const UPLOAD_ACK_TIMEOUT_MS = 10_000;
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

export function deepResearchStartDelayMs(randomValue = Math.random()): number {
  const jitter = Math.min(1, Math.max(0, randomValue));
  return (
    DEEP_RESEARCH_START_DELAY_MIN_MS +
    Math.round(jitter * DEEP_RESEARCH_START_DELAY_JITTER_MS)
  );
}
