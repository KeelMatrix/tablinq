import type { AskExecutionCommand, AskResultSnapshot } from "../contracts";
import {
  applyAskConfiguration,
  clickSubmit,
  fillComposer,
  isTransientPageError,
  readAskBaseline,
  readAskSnapshot,
  resetConversationIdentity,
  sleep,
  uploadAttachments,
  waitForSubmitReadyAfterUploads,
} from "../pageRuntime";
import {
  canFinalizeRecoveredAskSnapshot,
  computeStallTimeoutMs,
  textRelation,
} from "../snapshotState";
import {
  ASK_STALL_RECOVERY_GRACE_MS,
  ASK_TIMEOUT_MS,
  MAX_RECOVERY_ATTEMPTS,
  SNAPSHOT_POLL_INTERVAL_MS,
  STABLE_DONE_POLLS,
  SUBMIT_READY_AFTER_UPLOAD_TIMEOUT_MS,
} from "../timing";
import { conversationIdFromUrl } from "../urls";
import { safePageUrl } from "../capabilityProbe";

import { finalizeAsk, stabilizeAskCompletion } from "./finalization";
import { reconcileAskAfterInterruption } from "./askRecovery";
import { runResearch } from "./research";
import { applyAskSnapshot, rememberAskSnapshot } from "./streaming";
import type {
  AskExecutionOptions,
  AskRuntimeState,
  BrowserExecutionHost,
  ChunkSink,
  ResearchSelection,
  StateSink,
} from "./types";

const PROVIDER_GENERATION_FAILURE_CODE =
  "provider_generation_failed_nonretryable";

class AskProviderFailure extends Error {
  readonly code = PROVIDER_GENERATION_FAILURE_CODE;
  readonly currentUrl: string;
  readonly retryVisible: boolean;

  constructor(message: string, currentUrl: string, retryVisible: boolean) {
    super(message);
    this.name = "AskProviderFailure";
    this.currentUrl = currentUrl;
    this.retryVisible = retryVisible;
  }
}

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

  const timeoutAt = Date.now() + (command.timeoutMs ?? ASK_TIMEOUT_MS);
  const stallTimeoutMs = computeStallTimeoutMs(
    command.timeoutMs,
    ASK_TIMEOUT_MS,
  );
  const initialAskStallTimeoutMs = Math.max(
    stallTimeoutMs,
    Math.min(
      ASK_STALL_RECOVERY_GRACE_MS,
      Math.floor((command.timeoutMs ?? ASK_TIMEOUT_MS) / 2),
    ),
  );
  let page = await host.openExecutionPage(
    host.resolveAskTarget(command.conversationId, command.newConversation),
  );
  let askBaseline = null;

  if (options.submitPrompt) {
    if (
      (command.newConversation && !command.conversationId) ||
      (!command.conversationId && !conversationIdFromUrl(safePageUrl(page)))
    ) {
      await resetConversationIdentity(page);
    }
    askBaseline = await readAskBaseline(page);
    await host.refreshCapabilities(page);
    if (
      command.modelClass ||
      command.effort ||
      (command.mode && command.mode !== "normal")
    ) {
      page = await applyAskConfiguration(
        page,
        command,
        host.getLastCapabilityReport(),
      );
      await host.refreshCapabilities(page);
    }
    await uploadAttachments(
      page,
      command.files ?? [],
      host.getLastCapabilityReport(),
    );
    await fillComposer(page, command.prompt);
    if ((command.files ?? []).length > 0) {
      await waitForSubmitReadyAfterUploads(
        page,
        Math.min(
          SUBMIT_READY_AFTER_UPLOAD_TIMEOUT_MS,
          Math.max(1, timeoutAt - Date.now()),
        ),
      );
    }
    await clickSubmit(page);
  }

  const state: AskRuntimeState = {
    emittedText: options.emittedText,
    nextSeq: options.nextSeq,
    doneStreak: 0,
    knownConversationId:
      command.conversationId ??
      askBaseline?.conversationId ??
      conversationIdFromUrl(safePageUrl(page)) ??
      null,
    knownMessageId: null,
    pendingMessageId: null,
    pendingMessageText: "",
    askBaseline,
    pendingTextReconciliation: false,
    lastProgressAt: Date.now(),
    recoveryAttempts: 0,
    lastStatusAt: 0,
  };

  if (!options.submitPrompt) {
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
    page = recovered.page;
    if (recovered.result) {
      return recovered.result;
    }
    state.lastProgressAt = Date.now();
  }

  while (Date.now() < timeoutAt) {
    let snapshot;
    try {
      snapshot = await readAskSnapshot(page, state.askBaseline);
    } catch (error) {
      if (
        isTransientPageError(error) &&
        state.recoveryAttempts < MAX_RECOVERY_ATTEMPTS
      ) {
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
        page = recovered.page;
        if (recovered.result) {
          return recovered.result;
        }
        state.lastProgressAt = Date.now();
        await sleep(SNAPSHOT_POLL_INTERVAL_MS);
        continue;
      }
      throw error;
    }

    if (snapshot.failureCode === PROVIDER_GENERATION_FAILURE_CODE) {
      throw new AskProviderFailure(
        snapshot.failureMessage ??
          "Something went wrong while generating the response. If this issue persists please contact us through our help center.",
        snapshot.currentUrl,
        snapshot.retryVisible,
      );
    }

    rememberAskSnapshot(host, state, snapshot);
    if (
      state.pendingTextReconciliation &&
      textRelation(state.emittedText, snapshot.text) !== "mismatch"
    ) {
      state.pendingTextReconciliation = false;
    }
    const progressed = await applyAskSnapshot(
      command,
      state,
      snapshot,
      onChunk,
    );
    if (progressed) {
      state.lastProgressAt = Date.now();
    }

    if (snapshot.state === "done") {
      state.doneStreak += 1;
    } else {
      state.doneStreak = 0;
    }

    if (snapshot.state === "done" && state.doneStreak >= STABLE_DONE_POLLS) {
      snapshot = await stabilizeAskCompletion(
        page,
        state.askBaseline,
        snapshot,
        timeoutAt,
      );
      rememberAskSnapshot(host, state, snapshot);
      const stabilizedProgress = await applyAskSnapshot(
        command,
        state,
        snapshot,
        onChunk,
      );
      if (stabilizedProgress) {
        state.lastProgressAt = Date.now();
      }
      if (snapshot.state === "done") {
        if (
          state.pendingTextReconciliation &&
          !canFinalizeRecoveredAskSnapshot(
            state.emittedText,
            state.knownMessageId,
            snapshot,
          )
        ) {
          throw new Error(
            "interrupted ask could not be reconciled against the live conversation",
          );
        }
        state.pendingTextReconciliation = false;
        return finalizeAsk(host, command, page, state, snapshot, onChunk);
      }
      state.doneStreak = 0;
    }

    const canRecoverFromAskStall =
      state.emittedText.length > 0 ||
      state.knownConversationId !== null ||
      state.knownMessageId !== null;
    const askStallTimeoutMs =
      state.emittedText.length === 0 && state.knownMessageId === null
        ? initialAskStallTimeoutMs
        : stallTimeoutMs;
    if (
      Date.now() - state.lastProgressAt >= askStallTimeoutMs &&
      state.recoveryAttempts < MAX_RECOVERY_ATTEMPTS &&
      canRecoverFromAskStall
    ) {
      const recovered = await reconcileAskAfterInterruption(
        host,
        page,
        command,
        state,
        onChunk,
        onState,
        "stream stalled while waiting for provider output",
        timeoutAt,
      );
      page = recovered.page;
      if (recovered.result) {
        return recovered.result;
      }
      state.lastProgressAt = Date.now();
    }

    await sleep(SNAPSHOT_POLL_INTERVAL_MS);
  }

  throw new Error("ask timed out waiting for streamed response");
}

export async function runAskAsResearch(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  onChunk: ChunkSink,
  onState: StateSink,
  options: AskExecutionOptions,
): Promise<AskResultSnapshot> {
  let nextSeq = options.nextSeq;
  const result = await runResearch(
    host,
    {
      jobId: command.jobId,
      query: command.prompt,
      conversationId: command.conversationId,
      files: command.files,
      timeoutMs: command.timeoutMs,
    },
    async (update) => {
      if (update.chunk_kind !== "delta" || !update.chunk_text) {
        return;
      }
      await onChunk({
        job_id: command.jobId,
        conversation_id: update.conversation_id ?? null,
        message_id: update.message_id ?? null,
        seq: nextSeq,
        kind: "delta",
        text: update.chunk_text,
      });
      nextSeq += 1;
    },
    onState,
    options,
    {
      modelClass: command.modelClass ?? undefined,
      effort: command.effort ?? undefined,
    },
  );

  await onChunk({
    job_id: command.jobId,
    conversation_id: result.conversation_id,
    message_id: result.message_id,
    seq: nextSeq,
    kind: "done",
    text: "",
  });

  return {
    job_id: result.job_id,
    conversation_id: result.conversation_id,
    message_id: result.message_id,
    output_text: result.report_text,
    artifacts: result.artifacts,
    completed_at: result.completed_at,
    current_url: result.current_url,
  };
}
