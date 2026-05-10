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

export type ActionPacingProfile =
  | "micro"
  | "visual-choice"
  | "commit"
  | "retry"
  | "research-handoff";

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
  | "generation_failed"
  | "stopped_thinking";

interface PressureState {
  untilMs: number;
  signalCount: number;
  lastSignalAt: number;
}

interface ActionPacingBounds {
  profile: ActionPacingProfile;
  minMs: number;
  maxMs: number;
}

interface SessionRhythmState {
  consecutiveActions: number;
  lastActionAt: number;
}

export const DEEP_RESEARCH_START_DELAY_MIN_MS = 1_500;
export const DEEP_RESEARCH_START_DELAY_JITTER_MS = 3_500;
export const QUIET_SURFACE_TIMEOUT_MS = 1_500;
export const QUIET_SURFACE_STABLE_WINDOW_MS = 175;
const PRESSURE_SIGNAL_DURATIONS_MS: Record<PressureSignalKind, number> = {
  rate_limited: 20_000,
  overloaded: 15_000,
  retry_visible: 12_000,
  recovery_loop: 10_000,
  generation_failed: 18_000,
  stopped_thinking: 18_000,
};
const PRESSURE_RESET_WINDOW_MS = 20_000;
const MAX_PRESSURE_COOLDOWN_MS = 30_000;
const DETERMINISTIC_PACING_RANDOM_VALUE = 0.5;
const RHYTHM_IDLE_RESET_MS = 3_000;
const RHYTHM_STEP_BONUS = 0.045;
const RHYTHM_MAX_BONUS = 0.18;

const INTERACTION_PACING_BOUNDS: Record<
  InteractionJitterKind,
  ActionPacingBounds
> = {
  chip_clear: { profile: "micro", minMs: 30, maxMs: 80 },
  keyboard_submit: { profile: "micro", minMs: 35, maxMs: 80 },
  picker_trigger: { profile: "micro", minMs: 45, maxMs: 120 },
  choice_select: { profile: "micro", minMs: 55, maxMs: 145 },
  mode_toggle: { profile: "micro", minMs: 60, maxMs: 150 },
  upload_entry: { profile: "micro", minMs: 70, maxMs: 160 },
  button_click: { profile: "micro", minMs: 70, maxMs: 170 },
};

const HIGH_RISK_ACTION_PACING_BOUNDS: Record<
  HighRiskActionKind,
  ActionPacingBounds
> = {
  picker_open: { profile: "visual-choice", minMs: 150, maxMs: 350 },
  option_select: { profile: "visual-choice", minMs: 250, maxMs: 700 },
  mode_toggle: { profile: "visual-choice", minMs: 300, maxMs: 900 },
  upload_entry: { profile: "commit", minMs: 500, maxMs: 1_200 },
  final_submit: { profile: "commit", minMs: 700, maxMs: 1_800 },
  retry_click: { profile: "retry", minMs: 1_500, maxMs: 5_000 },
};

const DEEP_RESEARCH_HANDOFF_PACING_BOUNDS: ActionPacingBounds = {
  profile: "research-handoff",
  minMs: DEEP_RESEARCH_START_DELAY_MIN_MS,
  maxMs: DEEP_RESEARCH_START_DELAY_MIN_MS + DEEP_RESEARCH_START_DELAY_JITTER_MS,
};

const pressureState: PressureState = {
  untilMs: 0,
  signalCount: 0,
  lastSignalAt: 0,
};

const sessionRhythmState: SessionRhythmState = {
  consecutiveActions: 0,
  lastActionAt: 0,
};

export function deepResearchStartDelayMs(
  randomValue: number | string = Math.random(),
  enabled = shortInteractionJitterEnabled(),
  now = Date.now(),
): number {
  return actionPacingDelayMs(
    DEEP_RESEARCH_HANDOFF_PACING_BOUNDS,
    randomValue,
    enabled,
    now,
  );
}

export function interactionJitterMs(
  kind: InteractionJitterKind,
  randomValue: number | string = Math.random(),
  enabled = shortInteractionJitterEnabled(),
  now = Date.now(),
): number {
  return actionPacingDelayMs(
    INTERACTION_PACING_BOUNDS[kind],
    randomValue,
    enabled,
    now,
  );
}

export function shortInteractionJitterEnabled(): boolean {
  return (
    readBooleanEnv("TABLINQ_SHORT_INTERACTION_JITTER") ??
    process.env.VITEST !== "true"
  );
}

export function highRiskPacingEnabled(): boolean {
  return (
    readBooleanEnv("TABLINQ_HIGH_RISK_PACING") ?? process.env.VITEST !== "true"
  );
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
  randomValue: number | string = Math.random(),
  enabled = highRiskPacingEnabled(),
  now = Date.now(),
): number {
  return actionPacingDelayMs(
    HIGH_RISK_ACTION_PACING_BOUNDS[action],
    randomValue,
    enabled,
    now,
  );
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
  return widenDelayForActionPressure(action, baseMs, now, 250);
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
  return widenDelayForActionPressure(action, baseMs, now, 1_000);
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
    PRESSURE_SIGNAL_DURATIONS_MS[signal] +
      (pressureState.signalCount - 1) * 2_000,
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

export function resetSessionRhythmForTest(): void {
  sessionRhythmState.consecutiveActions = 0;
  sessionRhythmState.lastActionAt = 0;
}

export function pacingRandomValueFromSeed(seed: string): number {
  let hash = 2_166_136_261;
  for (let index = 0; index < seed.length; index += 1) {
    hash ^= seed.charCodeAt(index);
    hash = Math.imul(hash, 16_777_619);
  }
  return (hash >>> 0) / 0xffff_ffff;
}

function deterministicPacingEnabled(): boolean {
  return readBooleanEnv("TABLINQ_PACING_DETERMINISTIC") ?? false;
}

function actionPacingDelayMs(
  bounds: ActionPacingBounds,
  randomValue: number | string,
  enabled: boolean,
  now: number,
): number {
  if (!enabled) {
    return 0;
  }

  const jitter = resolvePacingRandomValue(randomValue);
  const baseMs = interpolate(bounds.minMs, bounds.maxMs, jitter);
  const rhythmicMs = applySessionRhythm(baseMs, bounds.maxMs, now);
  return widenDelayForProfilePressure(
    bounds.profile,
    rhythmicMs,
    now,
    providerPressureMaxBonusMs(bounds.profile),
  );
}

function resolvePacingRandomValue(randomValue: number | string): number {
  if (deterministicPacingEnabled()) {
    return DETERMINISTIC_PACING_RANDOM_VALUE;
  }
  if (typeof randomValue === "string") {
    return pacingRandomValueFromSeed(randomValue);
  }
  return Math.min(1, Math.max(0, randomValue));
}

function interpolate(minMs: number, maxMs: number, factor: number): number {
  return minMs + Math.round((maxMs - minMs) * factor);
}

function applySessionRhythm(
  baseMs: number,
  maxMs: number,
  now: number,
): number {
  if (baseMs <= 0) {
    return 0;
  }

  if (
    sessionRhythmState.lastActionAt <= 0 ||
    now - sessionRhythmState.lastActionAt > RHYTHM_IDLE_RESET_MS
  ) {
    sessionRhythmState.consecutiveActions = 0;
  }

  const bonus = Math.min(
    RHYTHM_MAX_BONUS,
    sessionRhythmState.consecutiveActions * RHYTHM_STEP_BONUS,
  );
  sessionRhythmState.consecutiveActions = Math.min(
    4,
    sessionRhythmState.consecutiveActions + 1,
  );
  sessionRhythmState.lastActionAt = now;

  return Math.min(maxMs, Math.round(baseMs * (1 + bonus)));
}

function providerPressureMaxBonusMs(profile: ActionPacingProfile): number {
  switch (profile) {
    case "commit":
      return 600;
    case "retry":
      return 2_500;
    case "research-handoff":
      return 3_000;
    case "micro":
    case "visual-choice":
      return 0;
  }
}

function widenDelayForActionPressure(
  action: HighRiskActionKind,
  baseMs: number,
  now: number,
  maxBonusMs: number,
): number {
  return widenDelayForProfilePressure(
    HIGH_RISK_ACTION_PACING_BOUNDS[action].profile,
    baseMs,
    now,
    maxBonusMs,
  );
}

function widenDelayForProfilePressure(
  profile: ActionPacingProfile,
  baseMs: number,
  now: number,
  maxBonusMs: number,
): number {
  if (maxBonusMs <= 0 || providerPressureMaxBonusMs(profile) <= 0) {
    return baseMs;
  }
  return widenDelayForProviderPressure(baseMs, now, maxBonusMs);
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
