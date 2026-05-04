import type { AskExecutionCommand, AskResultSnapshot } from "../contracts";
import { readAskSnapshot, scrollToBottomIfNeeded } from "../pageRuntime";
import { safePageUrl } from "../capabilityProbe";
import {
  normalizeComparableAskText,
  type AskBaseline,
  type AskSnapshot,
} from "../snapshotState";
import { NAVIGATION_TIMEOUT_MS } from "../timing";

import { throwIfProviderGenerationFailed } from "./askProviderFailure";
import {
  recoverAskSnapshotRead,
  recoverInitialAskIfNeeded,
  recoverStalledAskIfNeeded,
  handleStickyStreamingAskIfNeeded,
} from "./askRecoveryFlow";
import { openAskExecutionPage, prepareAskSubmission } from "./askSubmission";
import { finalizeAsk, stabilizeAskCompletion } from "./finalization";
import {
  createAskRuntimeState,
  emitAskIdentityUpdate,
  finalizeStableAskIfReady,
  recordAskSnapshotProgress,
  updateAskDoneStreak,
} from "./askState";
import { type AskTiming, resolveAskTiming, sleepForAskPoll } from "./askTiming";
import type {
  AskExecutionOptions,
  BrowserExecutionHost,
  ChunkSink,
  StateSink,
} from "./types";

export async function runAsk(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  onChunk: ChunkSink,
  onState: StateSink,
  options: AskExecutionOptions,
): Promise<AskResultSnapshot> {
  let timing = resolveAskTiming(command);
  let page = await openAskExecutionPage(host, command);
  await scrollToBottomIfNeeded(page);
  let askBaseline: AskBaseline | null = null;
  const responseWatchdog = createAskResponseStreamWatchdog(timing);

  if (options.submitPrompt) {
    const prepared = await prepareAskSubmission(host, command, page, timing);
    page = prepared.page;
    askBaseline = prepared.askBaseline;
    timing = {
      ...timing,
      timeoutAt: Date.now() + timing.executionTimeoutMs,
    };
  }

  const state = createAskRuntimeState(command, options, page, askBaseline);
  await emitAskIdentityUpdate(
    command,
    state,
    safePageUrl(page),
    onState,
    "observed ask baseline",
  );

  const initialRecovery = await recoverInitialAskIfNeeded(
    host,
    command,
    page,
    state,
    onChunk,
    onState,
    options,
    timing.timeoutAt,
  );
  page = initialRecovery.page;
  if (initialRecovery.result) {
    return initialRecovery.result;
  }

  while (Date.now() < askActiveDeadline(responseWatchdog, timing.timeoutAt)) {
    const activeDeadline = askActiveDeadline(
      responseWatchdog,
      timing.timeoutAt,
    );
    let snapshot: AskSnapshot;
    try {
      snapshot = await readAskSnapshot(page, state.askBaseline);
    } catch (error) {
      const recovered = await recoverAskSnapshotRead(
        host,
        command,
        page,
        state,
        onChunk,
        onState,
        error,
        activeDeadline,
      );
      if (recovered) {
        page = recovered.page;
        if (recovered.result) {
          return recovered.result;
        }
        continue;
      }
      throw error;
    }

    throwIfProviderGenerationFailed(snapshot);

    await recordAskSnapshotProgress(
      host,
      command,
      state,
      snapshot,
      onChunk,
      onState,
      "observed in-flight assistant identity",
    );
    updateAskDoneStreak(state, snapshot);

    const finalized = await finalizeStableAskIfReady(
      host,
      command,
      page,
      state,
      snapshot,
      onChunk,
      onState,
      timing.timeoutAt,
    );
    snapshot = finalized.snapshot;
    if (finalized.result) {
      return finalized.result;
    }

    const streamWatchdogResult = await handleAskResponseStreamWatchdog(
      host,
      command,
      page,
      state,
      snapshot,
      onChunk,
      onState,
      responseWatchdog,
    );
    page = streamWatchdogResult.page;
    if (streamWatchdogResult.result) {
      return streamWatchdogResult.result;
    }
    if (
      streamWatchdogResult.active &&
      timing.stickyStreamReloadStallMs >= responseWatchdog.snapshotIntervalMs
    ) {
      await sleepForAskPoll(
        state,
        askActiveDeadline(responseWatchdog, timing.timeoutAt),
      );
      continue;
    }

    const stickyStreamCompletion = await handleStickyStreamingAskIfNeeded(
      host,
      command,
      page,
      state,
      snapshot,
      onChunk,
      onState,
      timing,
    );
    page = stickyStreamCompletion.page;
    if (stickyStreamCompletion.result) {
      return stickyStreamCompletion.result;
    }

    if (state.stickyStreamFallbackAt === null) {
      const stallRecovery = await recoverStalledAskIfNeeded(
        host,
        command,
        page,
        state,
        onChunk,
        onState,
        timing,
      );
      page = stallRecovery.page;
      if (stallRecovery.result) {
        return stallRecovery.result;
      }
    }

    await sleepForAskPoll(state, timing.timeoutAt);
  }

  if (responseWatchdog.startedAt !== null) {
    throw new Error("ask timed out while streaming response");
  }

  const timeoutReload = await reloadAskOnceAfterTimeout(
    host,
    command,
    page,
    state,
    onChunk,
    onState,
  );
  if (timeoutReload.result) {
    return timeoutReload.result;
  }

  throw new Error("ask timed out waiting for streamed response");
}

type AskResponseStreamWatchdog = {
  startedAt: number | null;
  maxAt: number | null;
  snapshotIntervalMs: number;
  maxStreamMs: number;
  nextSnapshotAt: number | null;
  lastSnapshotText: string;
  reloadAttempts: number;
};

function createAskResponseStreamWatchdog(
  timing: AskTiming,
): AskResponseStreamWatchdog {
  return {
    startedAt: null,
    maxAt: null,
    snapshotIntervalMs: timing.responseStreamSnapshotIntervalMs,
    maxStreamMs: timing.responseStreamMaxMs,
    nextSnapshotAt: null,
    lastSnapshotText: "",
    reloadAttempts: 0,
  };
}

function askActiveDeadline(
  watchdog: AskResponseStreamWatchdog,
  fallbackTimeoutAt: number,
): number {
  return watchdog.maxAt ?? fallbackTimeoutAt;
}

async function handleAskResponseStreamWatchdog(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  page: Awaited<ReturnType<typeof openAskExecutionPage>>,
  state: ReturnType<typeof createAskRuntimeState>,
  snapshot: AskSnapshot,
  onChunk: ChunkSink,
  onState: StateSink,
  watchdog: AskResponseStreamWatchdog,
): Promise<{ page: typeof page; result?: AskResultSnapshot; active: boolean }> {
  const snapshotText = askResponseStreamWatchText(snapshot);
  if (!snapshotText) {
    return { page, active: watchdog.startedAt !== null };
  }

  const now = Date.now();
  if (watchdog.startedAt === null) {
    watchdog.startedAt = now;
    watchdog.maxAt = now + watchdog.maxStreamMs;
    watchdog.lastSnapshotText = snapshotText;
    watchdog.nextSnapshotAt = now + watchdog.snapshotIntervalMs;
    return { page, active: true };
  }

  if (now < (watchdog.nextSnapshotAt ?? now)) {
    return { page, active: true };
  }

  if (snapshotText !== watchdog.lastSnapshotText) {
    watchdog.lastSnapshotText = snapshotText;
    watchdog.nextSnapshotAt = now + watchdog.snapshotIntervalMs;
    return { page, active: true };
  }

  if (watchdog.reloadAttempts >= 3) {
    throw new Error("ask timed out while streaming response");
  }

  await onState({
    state: "interrupted",
    reason: "ask response stream stopped changing; reloading once",
    at: new Date().toISOString(),
    conversation_id: state.knownConversationId,
    message_id: state.knownMessageId,
    current_url: snapshot.currentUrl,
    streaming_safe: false,
  });
  await page
    .reload({ waitUntil: "domcontentloaded", timeout: NAVIGATION_TIMEOUT_MS })
    .catch(() => undefined);
  await scrollToBottomIfNeeded(page).catch(() => undefined);

  const reloadedSnapshot = await readAskSnapshot(page, state.askBaseline);
  throwIfProviderGenerationFailed(reloadedSnapshot);
  await recordAskSnapshotProgress(
    host,
    command,
    state,
    reloadedSnapshot,
    onChunk,
    onState,
    "observed assistant identity after stream watchdog reload",
  );
  updateAskDoneStreak(state, reloadedSnapshot);

  const reloadedText = askResponseStreamWatchText(reloadedSnapshot);
  if (!reloadedText || reloadedText === watchdog.lastSnapshotText) {
    throw new Error("ask timed out while streaming response");
  }

  watchdog.reloadAttempts += 1;
  watchdog.lastSnapshotText = reloadedText;
  watchdog.nextSnapshotAt = Date.now() + watchdog.snapshotIntervalMs;

  if (reloadedSnapshot.state !== "done") {
    return { page, active: true };
  }

  const stabilized = await stabilizeAskCompletion(
    page,
    state.askBaseline,
    reloadedSnapshot,
    askActiveDeadline(watchdog, Date.now() + 1_000),
  );
  await recordAskSnapshotProgress(
    host,
    command,
    state,
    stabilized,
    onChunk,
    onState,
    "observed stabilized assistant identity after stream watchdog reload",
  );
  if (stabilized.state !== "done") {
    return { page, active: true };
  }

  return {
    page,
    active: true,
    result: await finalizeAsk(host, command, page, state, stabilized, onChunk),
  };
}

function askResponseStreamWatchText(snapshot: AskSnapshot): string {
  if (!snapshot.hasAssistantMessage) {
    return "";
  }
  return normalizeComparableAskText(snapshot.text);
}

async function reloadAskOnceAfterTimeout(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  page: Awaited<ReturnType<typeof openAskExecutionPage>>,
  state: ReturnType<typeof createAskRuntimeState>,
  onChunk: ChunkSink,
  onState: StateSink,
): Promise<{ result?: AskResultSnapshot }> {
  await onState({
    state: "interrupted",
    reason: "ask reached its timeout; reloading once before failing",
    at: new Date().toISOString(),
    conversation_id: state.knownConversationId,
    message_id: state.knownMessageId,
    current_url: safePageUrl(page),
    streaming_safe: false,
  });
  await page
    .reload({ waitUntil: "domcontentloaded", timeout: NAVIGATION_TIMEOUT_MS })
    .catch(() => undefined);
  await scrollToBottomIfNeeded(page).catch(() => undefined);
  const snapshot = await readAskSnapshot(page, state.askBaseline).catch(
    () => null,
  );
  if (!snapshot) {
    return {};
  }
  throwIfProviderGenerationFailed(snapshot);
  await recordAskSnapshotProgress(
    host,
    command,
    state,
    snapshot,
    onChunk,
    onState,
    "observed assistant identity after timeout reload",
  );
  updateAskDoneStreak(state, snapshot);
  if (snapshot.state !== "done") {
    return {};
  }
  const stabilized = await stabilizeAskCompletion(
    page,
    state.askBaseline,
    snapshot,
    Date.now() + 1_000,
  );
  await recordAskSnapshotProgress(
    host,
    command,
    state,
    stabilized,
    onChunk,
    onState,
    "observed stabilized assistant identity after timeout reload",
  );
  if (stabilized.state !== "done") {
    return {};
  }
  return {
    result: await finalizeAsk(host, command, page, state, stabilized, onChunk),
  };
}
