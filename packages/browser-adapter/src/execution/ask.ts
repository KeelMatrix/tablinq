import type { AskExecutionCommand, AskResultSnapshot } from "../contracts";
import { readAskSnapshot, scrollToBottomIfNeeded } from "../pageRuntime";
import { safePageUrl } from "../capabilityProbe";
import type { AskBaseline, AskSnapshot } from "../snapshotState";

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
import { resolveAskTiming, sleepForAskPoll } from "./askTiming";
import { NAVIGATION_TIMEOUT_MS } from "../timing";
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
  const timing = resolveAskTiming(command);
  let page = await openAskExecutionPage(host, command);
  await scrollToBottomIfNeeded(page);
  let askBaseline: AskBaseline | null = null;

  if (options.submitPrompt) {
    const prepared = await prepareAskSubmission(host, command, page, timing);
    page = prepared.page;
    askBaseline = prepared.askBaseline;
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

  while (Date.now() < timing.timeoutAt) {
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
        timing.timeoutAt,
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
