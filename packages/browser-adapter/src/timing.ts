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
export const QUIET_SURFACE_TIMEOUT_MS = 1_500;
export const QUIET_SURFACE_STABLE_WINDOW_MS = 175;

export type InteractionJitterKind =
  | "chip_clear"
  | "picker_trigger"
  | "choice_select"
  | "mode_toggle"
  | "button_click"
  | "keyboard_submit"
  | "upload_entry";

export type HighRiskActionKind =
  | "picker_open"
  | "option_select"
  | "mode_toggle"
  | "upload_entry"
  | "final_submit"
  | "retry_click";

export type AdaptivePollingProfile =
  | "edge"
  | "stabilize"
  | "upload"
  | "generation"
  | "recovery";

export type PressureSignalKind =
  | "rate_limited"
  | "overloaded"
  | "retry_visible"
  | "recovery_loop"
  | "generation_failed";

interface PressureState {
  untilMs: number;
  signalCount: number;
  lastSignalAt: number;
}

const PRESSURE_SIGNAL_DURATIONS_MS: Record<PressureSignalKind, number> = {
  rate_limited: 20_000,
  overloaded: 15_000,
  retry_visible: 12_000,
  recovery_loop: 10_000,
  generation_failed: 18_000,
};
const PRESSURE_RESET_WINDOW_MS = 20_000;
const MAX_PRESSURE_COOLDOWN_MS = 30_000;
const DETERMINISTIC_PACING_RANDOM_VALUE = 0.5;

const pressureState: PressureState = {
  untilMs: 0,
  signalCount: 0,
  lastSignalAt: 0,
};

export function deepResearchStartDelayMs(
  randomValue = Math.random(),
  enabled = shortInteractionJitterEnabled(),
): number {
  if (!enabled) {
    return 0;
  }
  const jitter = resolvePacingRandomValue(randomValue);
  return (
    DEEP_RESEARCH_START_DELAY_MIN_MS +
    Math.round(jitter * DEEP_RESEARCH_START_DELAY_JITTER_MS)
  );
}

export function interactionJitterMs(
  kind: InteractionJitterKind,
  randomValue = Math.random(),
  enabled = shortInteractionJitterEnabled(),
): number {
  if (!enabled) {
    return 0;
  }
  const jitter = resolvePacingRandomValue(randomValue);
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

export function shortInteractionJitterEnabled(): boolean {
  return readBooleanEnv("TABLINQ_SHORT_INTERACTION_JITTER") ?? process.env.VITEST !== "true";
}

export function highRiskPacingEnabled(): boolean {
  return readBooleanEnv("TABLINQ_HIGH_RISK_PACING") ?? process.env.VITEST !== "true";
}

export function highRiskActionForInteractionKind(
  kind: InteractionJitterKind,
): HighRiskActionKind | null {
  switch (kind) {
    case "picker_trigger":
      return "picker_open";
    case "choice_select":
      return "option_select";
    case "mode_toggle":
      return "mode_toggle";
    case "upload_entry":
      return "upload_entry";
    case "chip_clear":
    case "button_click":
    case "keyboard_submit":
      return null;
  }
}

export function highRiskActionCooldownMs(
  action: HighRiskActionKind,
  randomValue = Math.random(),
  enabled = highRiskPacingEnabled(),
  now = Date.now(),
): number {
  if (!enabled) {
    return 0;
  }

  const jitter = resolvePacingRandomValue(randomValue);
  let delayMs: number;
  switch (action) {
    case "picker_open":
      delayMs = interpolate(70, 130, jitter);
      break;
    case "option_select":
      delayMs = interpolate(80, 150, jitter);
      break;
    case "mode_toggle":
      delayMs = interpolate(90, 170, jitter);
      break;
    case "upload_entry":
      delayMs = interpolate(110, 220, jitter);
      break;
    case "final_submit":
      delayMs = interpolate(100, 180, jitter);
      break;
    case "retry_click":
      delayMs = interpolate(180, 320, jitter);
      break;
  }

  return widenDelayForProviderPressure(delayMs, now, 600);
}

export function quietSurfaceStableWindowMs(
  action: HighRiskActionKind,
  now = Date.now(),
): number {
  let baseMs: number;
  switch (action) {
    case "picker_open":
    case "option_select":
      baseMs = QUIET_SURFACE_STABLE_WINDOW_MS;
      break;
    case "mode_toggle":
    case "final_submit":
      baseMs = 200;
      break;
    case "upload_entry":
    case "retry_click":
      baseMs = 225;
      break;
  }
  return widenDelayForProviderPressure(baseMs, now, 250);
}

export function quietSurfaceTimeoutMs(
  action: HighRiskActionKind,
  now = Date.now(),
): number {
  let baseMs: number;
  switch (action) {
    case "picker_open":
    case "option_select":
      baseMs = QUIET_SURFACE_TIMEOUT_MS;
      break;
    case "mode_toggle":
    case "upload_entry":
    case "final_submit":
      baseMs = 2_000;
      break;
    case "retry_click":
      baseMs = 2_500;
      break;
  }
  return widenDelayForProviderPressure(baseMs, now, 1_000);
}

export function adaptivePollIntervalMs(
  profile: AdaptivePollingProfile,
  elapsedMs: number,
  now = Date.now(),
): number {
  const waitedMs = Math.max(0, Math.trunc(elapsedMs));
  let intervalMs: number;

  switch (profile) {
    case "edge":
      if (waitedMs < 400) {
        intervalMs = 50;
      } else if (waitedMs < 1_500) {
        intervalMs = 85;
      } else {
        intervalMs = 125;
      }
      break;
    case "stabilize":
      if (waitedMs < 800) {
        intervalMs = 75;
      } else if (waitedMs < 2_500) {
        intervalMs = 110;
      } else {
        intervalMs = 160;
      }
      break;
    case "upload":
      if (waitedMs < 1_000) {
        intervalMs = 75;
      } else if (waitedMs < 4_000) {
        intervalMs = 150;
      } else {
        intervalMs = 275;
      }
      break;
    case "generation":
      if (waitedMs < 1_000) {
        intervalMs = 75;
      } else if (waitedMs < 5_000) {
        intervalMs = 175;
      } else {
        intervalMs = 350;
      }
      break;
    case "recovery":
      if (waitedMs < 1_000) {
        intervalMs = 85;
      } else if (waitedMs < 4_000) {
        intervalMs = 150;
      } else {
        intervalMs = 250;
      }
      break;
  }

  const remainingPressureMs = providerPressureCooldownMs(now);
  if (remainingPressureMs <= 0) {
    return intervalMs;
  }
  return Math.min(
    750,
    intervalMs + Math.min(250, Math.round(remainingPressureMs / 120)),
  );
}

export function noteProviderPressure(
  signal: PressureSignalKind,
  now = Date.now(),
): number {
  if (now - pressureState.lastSignalAt > PRESSURE_RESET_WINDOW_MS) {
    pressureState.signalCount = 0;
  }
  pressureState.signalCount = Math.min(4, pressureState.signalCount + 1);
  pressureState.lastSignalAt = now;

  const durationMs = Math.min(
    MAX_PRESSURE_COOLDOWN_MS,
    PRESSURE_SIGNAL_DURATIONS_MS[signal] + (pressureState.signalCount - 1) * 2_000,
  );
  pressureState.untilMs = Math.max(pressureState.untilMs, now + durationMs);
  return providerPressureCooldownMs(now);
}

export function noteProviderPressureFromText(
  message: string | null | undefined,
  now = Date.now(),
): number {
  const signals = inferPressureSignalsFromText(message);
  let remainingMs = providerPressureCooldownMs(now);
  for (const signal of signals) {
    remainingMs = Math.max(remainingMs, noteProviderPressure(signal, now));
  }
  return remainingMs;
}

export function providerPressureCooldownMs(now = Date.now()): number {
  if (pressureState.untilMs <= now) {
    pressureState.untilMs = 0;
    pressureState.signalCount = 0;
    return 0;
  }
  return pressureState.untilMs - now;
}

export function resetProviderPressureForTest(): void {
  pressureState.untilMs = 0;
  pressureState.signalCount = 0;
  pressureState.lastSignalAt = 0;
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

function deterministicPacingEnabled(): boolean {
  return readBooleanEnv("TABLINQ_PACING_DETERMINISTIC") ?? false;
}

function resolvePacingRandomValue(randomValue: number): number {
  if (deterministicPacingEnabled()) {
    return DETERMINISTIC_PACING_RANDOM_VALUE;
  }
  return Math.min(1, Math.max(0, randomValue));
}

function interpolate(minMs: number, maxMs: number, factor: number): number {
  return minMs + Math.round((maxMs - minMs) * factor);
}

function widenDelayForProviderPressure(
  baseMs: number,
  now: number,
  maxBonusMs: number,
): number {
  if (baseMs <= 0) {
    return 0;
  }

  const remainingPressureMs = providerPressureCooldownMs(now);
  if (remainingPressureMs <= 0) {
    return baseMs;
  }

  const bonusMs = Math.min(
    maxBonusMs,
    Math.max(Math.round(baseMs * 0.4), Math.round(remainingPressureMs / 40)),
  );
  return baseMs + bonusMs;
}

function readBooleanEnv(name: string): boolean | undefined {
  const value = process.env[name]?.trim().toLowerCase();
  if (value === "true") {
    return true;
  }
  if (value === "false") {
    return false;
  }
  return undefined;
}

function inferPressureSignalsFromText(
  message: string | null | undefined,
): PressureSignalKind[] {
  const normalized = String(message ?? "").toLowerCase();
  const signals: PressureSignalKind[] = [];
  if (
    normalized.includes("429") ||
    normalized.includes("rate limit") ||
    normalized.includes("too many requests")
  ) {
    signals.push("rate_limited");
  }
  if (
    normalized.includes("overloaded") ||
    normalized.includes("queue full") ||
    normalized.includes("queue is full") ||
    normalized.includes("at capacity") ||
    normalized.includes("try again later")
  ) {
    signals.push("overloaded");
  }
  return signals;
}
