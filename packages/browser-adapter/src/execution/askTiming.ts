
import type { AskExecutionCommand } from "../contracts";
import { sleep } from "../pageRuntime";
import { computeStallTimeoutMs } from "../snapshotState";
import {
  adaptivePollIntervalMs,
  ASK_STALL_RECOVERY_GRACE_MS,
  ASK_TIMEOUT_MS,
  askStickyStreamFinalGraceMs,
  askStickyStreamReloadStallMs,
} from "../timing";

import type { AskRuntimeState } from "./types";

export type AskTiming = {
  timeoutAt: number;
  stallTimeoutMs: number;
  initialStallTimeoutMs: number;
  stickyStreamReloadStallMs: number;
  stickyStreamFinalGraceMs: number;
};

export function resolveAskTiming(command: AskExecutionCommand): AskTiming {
  const stickyStreamReloadStallMs = askStickyStreamReloadStallMs();
  const stickyStreamFinalGraceMs = askStickyStreamFinalGraceMs();
  const executionTimeoutMs =
    command.timeoutMs ??
    Math.max(
      ASK_TIMEOUT_MS,
      stickyStreamReloadStallMs + stickyStreamFinalGraceMs + 5_000,
    );
  const stallTimeoutMs = computeStallTimeoutMs(
    command.timeoutMs,
    ASK_TIMEOUT_MS,
  );
  return {
    timeoutAt: Date.now() + executionTimeoutMs,
    stallTimeoutMs,
    initialStallTimeoutMs: Math.max(
      stallTimeoutMs,
      Math.min(ASK_STALL_RECOVERY_GRACE_MS, Math.floor(executionTimeoutMs / 2)),
    ),
    stickyStreamReloadStallMs,
    stickyStreamFinalGraceMs,
  };
}

export async function sleepForAskRecovery(
  state: AskRuntimeState,
  timeoutAt: number,
): Promise<void> {
  await sleep(
    Math.min(
      adaptivePollIntervalMs("recovery", Date.now() - state.lastProgressAt),
      Math.max(1, timeoutAt - Date.now()),
    ),
  );
}

export async function sleepForAskPoll(
  state: AskRuntimeState,
  timeoutAt: number,
): Promise<void> {
  await sleep(
    Math.min(
      adaptivePollIntervalMs("generation", Date.now() - state.lastProgressAt),
      Math.max(1, timeoutAt - Date.now()),
    ),
  );
}
