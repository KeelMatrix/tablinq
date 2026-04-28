
import type {
  AskExecutionCommand,
  AskResultSnapshot,
} from "../contracts";
import { readAskSnapshot, scrollToBottomIfNeeded } from "../pageRuntime";
import { safePageUrl } from "../capabilityProbe";
import type { AskBaseline, AskSnapshot } from "../snapshotState";

import { runAskAsResearch } from "./askAsResearch";
import { throwIfProviderGenerationFailed } from "./askProviderFailure";
import {
  recoverAskSnapshotRead,
  recoverInitialAskIfNeeded,
  recoverStalledAskIfNeeded,
  handleStickyStreamingAskIfNeeded,
} from "./askRecoveryFlow";
import { openAskExecutionPage, prepareAskSubmission } from "./askSubmission";
import {
  createAskRuntimeState,
  emitAskIdentityUpdate,
  finalizeStableAskIfReady,
  recordAskSnapshotProgress,
  updateAskDoneStreak,
} from "./askState";
import { resolveAskTiming, sleepForAskPoll } from "./askTiming";
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
  if (command.mode === "research") {
    return runAskAsResearch(host, command, onChunk, onState, options);
  }

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

  throw new Error("ask timed out waiting for streamed response");
}
