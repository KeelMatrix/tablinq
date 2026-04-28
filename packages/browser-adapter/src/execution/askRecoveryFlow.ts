import type { Page } from "playwright-core";

import type { AskExecutionCommand } from "../contracts";
import { isTransientPageError, readAskSnapshot } from "../pageRuntime";
import type { AskSnapshot } from "../snapshotState";
import {
  MAX_RECOVERY_ATTEMPTS,
  NAVIGATION_TIMEOUT_MS,
  noteProviderPressure,
} from "../timing";

import { reconcileAskAfterInterruption } from "./askRecovery";
import { throwIfProviderGenerationFailed } from "./askProviderFailure";
import type { AskTiming } from "./askTiming";
import { sleepForAskRecovery } from "./askTiming";
import { finalizeAsk, stabilizeAskCompletion } from "./finalization";
import { recordAskSnapshotProgress, updateAskDoneStreak } from "./askState";
import type {
  AskExecutionOptions,
  AskRecoveryResult,
  AskRuntimeState,
  BrowserExecutionHost,
  ChunkSink,
  StateSink,
} from "./types";

export async function recoverInitialAskIfNeeded(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  page: Page,
  state: AskRuntimeState,
  onChunk: ChunkSink,
  onState: StateSink,
  options: AskExecutionOptions,
  timeoutAt: number,
): Promise<AskRecoveryResult> {
  if (options.submitPrompt) {
    return { page };
  }
  const recovered = await reconcileAskAfterInterruption(
    host,
    page,
    command,
    state,
    onChunk,
    onState,
    options.recoveryReason ?? "recovered the interrupted ask",
    timeoutAt,
  );
  if (!recovered.result) {
    state.lastProgressAt = Date.now();
  }
  return recovered;
}

export async function recoverAskSnapshotRead(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  page: Page,
  state: AskRuntimeState,
  onChunk: ChunkSink,
  onState: StateSink,
  error: unknown,
  timeoutAt: number,
): Promise<AskRecoveryResult | null> {
  if (
    !isTransientPageError(error) ||
    state.recoveryAttempts >= MAX_RECOVERY_ATTEMPTS
  ) {
    return null;
  }
  noteProviderPressure("recovery_loop");
  const recovered = await reconcileAskAfterInterruption(
    host,
    page,
    command,
    state,
    onChunk,
    onState,
    error instanceof Error ? error.message : String(error),
    timeoutAt,
  );
  if (!recovered.result) {
    state.lastProgressAt = Date.now();
    await sleepForAskRecovery(state, timeoutAt);
  }
  return recovered;
}

export async function handleStickyStreamingAskIfNeeded(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  page: Page,
  state: AskRuntimeState,
  snapshot: AskSnapshot,
  onChunk: ChunkSink,
  onState: StateSink,
  timing: AskTiming,
): Promise<AskRecoveryResult> {
  if (snapshot.state === "done" || state.lastObservedText.length === 0) {
    return { page };
  }

  const now = Date.now();
  if (
    state.stickyStreamFallbackAt !== null &&
    now >= state.stickyStreamFallbackAt
  ) {
    state.pendingTextReconciliation = false;
    return {
      page,
      result: await finalizeAsk(host, command, page, state, snapshot, onChunk),
    };
  }

  if (now - state.lastTextChangeAt < timing.stickyStreamReloadStallMs) {
    return { page };
  }

  if (state.stickyStreamReloaded) {
    state.stickyStreamFallbackAt =
      state.stickyStreamFallbackAt ?? now + timing.stickyStreamFinalGraceMs;
    return { page };
  }

  state.stickyStreamReloaded = true;
  await onState({
    state: "interrupted",
    reason:
      "stream showed no new assistant text while the provider still looked busy",
    at: new Date().toISOString(),
    conversation_id: state.knownConversationId,
    message_id: state.knownMessageId,
    current_url: snapshot.currentUrl,
    streaming_safe: false,
  });
  await page.reload({
    waitUntil: "domcontentloaded",
    timeout: NAVIGATION_TIMEOUT_MS,
  });

  const reloadedSnapshot = await readAskSnapshot(page, state.askBaseline);
  throwIfProviderGenerationFailed(reloadedSnapshot);
  await recordAskSnapshotProgress(
    host,
    command,
    state,
    reloadedSnapshot,
    onChunk,
    onState,
    "observed assistant reply after sticky stream reload",
  );
  updateAskDoneStreak(state, reloadedSnapshot);

  if (reloadedSnapshot.state === "done") {
    const stabilized = await stabilizeAskCompletion(
      page,
      state.askBaseline,
      reloadedSnapshot,
      timing.timeoutAt,
    );
    await recordAskSnapshotProgress(
      host,
      command,
      state,
      stabilized,
      onChunk,
      onState,
      "observed stabilized assistant identity after sticky stream reload",
    );
    if (stabilized.state === "done") {
      state.pendingTextReconciliation = false;
      return {
        page,
        result: await finalizeAsk(
          host,
          command,
          page,
          state,
          stabilized,
          onChunk,
        ),
      };
    }
  }

  state.stickyStreamFallbackAt = Date.now() + timing.stickyStreamFinalGraceMs;
  return { page };
}

export async function recoverStalledAskIfNeeded(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  page: Page,
  state: AskRuntimeState,
  onChunk: ChunkSink,
  onState: StateSink,
  timing: AskTiming,
): Promise<AskRecoveryResult> {
  const canRecoverFromAskStall =
    state.emittedText.length > 0 ||
    state.knownConversationId !== null ||
    state.knownMessageId !== null;
  const askStallTimeoutMs =
    state.emittedText.length === 0 && state.knownMessageId === null
      ? timing.initialStallTimeoutMs
      : timing.stallTimeoutMs;
  if (
    Date.now() - state.lastProgressAt < askStallTimeoutMs ||
    state.recoveryAttempts >= MAX_RECOVERY_ATTEMPTS ||
    !canRecoverFromAskStall
  ) {
    return { page };
  }
  noteProviderPressure("recovery_loop");
  const recovered = await reconcileAskAfterInterruption(
    host,
    page,
    command,
    state,
    onChunk,
    onState,
    "stream stalled while waiting for provider output",
    timing.timeoutAt,
  );
  if (!recovered.result) {
    state.lastProgressAt = Date.now();
  }
  return recovered;
}
