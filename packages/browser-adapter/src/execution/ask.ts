import type {
  AskExecutionCommand,
  AskResultSnapshot,
  RequestStateSnapshot,
} from "../contracts";
import type { Page } from "playwright-core";
import {
  applyAskConfiguration,
  clickSubmit,
  fillComposer,
  ensureConfiguredSearchMode,
  isTransientPageError,
  readAskBaseline,
  readAskSnapshot,
  resetConversationIdentity,
  scrollToBottomIfNeeded,
  sleep,
  uploadAttachments,
  waitForInteractionSurface,
  waitForSubmitReadyAfterUploads,
} from "../pageRuntime";
import {
  computeStallTimeoutMs,
  type AskBaseline,
  type AskSnapshot,
} from "../snapshotState";
import {
  adaptivePollIntervalMs,
  ASK_STALL_RECOVERY_GRACE_MS,
  ASK_TIMEOUT_MS,
  MAX_RECOVERY_ATTEMPTS,
  STABLE_DONE_POLLS,
  SUBMIT_READY_AFTER_UPLOAD_TIMEOUT_MS,
  noteProviderPressure,
  noteProviderPressureFromText,
} from "../timing";
import { conversationIdFromUrl } from "../urls";
import { safePageUrl } from "../capabilityProbe";

import { finalizeAsk, stabilizeAskCompletion } from "./finalization";
import { reconcileAskAfterInterruption } from "./askRecovery";
import { runResearch } from "./research";
import { applyAskSnapshot, rememberAskSnapshot } from "./streaming";
import type {
  AskExecutionOptions,
  AskRecoveryResult,
  AskRuntimeState,
  BrowserExecutionHost,
  ChunkSink,
  ResearchSelection,
  StateSink,
} from "./types";

const PROVIDER_GENERATION_FAILURE_CODE =
  "provider_generation_failed_nonretryable";
const PROVIDER_NETWORK_PREFLIGHT_TIMEOUT_MS = 1_500;

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

    await sleepForAskPoll(state, timing.timeoutAt);
  }

  throw new Error("ask timed out waiting for streamed response");
}

type AskTiming = {
  timeoutAt: number;
  stallTimeoutMs: number;
  initialStallTimeoutMs: number;
};

function resolveAskTiming(command: AskExecutionCommand): AskTiming {
  const executionTimeoutMs = command.timeoutMs ?? ASK_TIMEOUT_MS;
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
  };
}

async function openAskExecutionPage(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
): Promise<Page> {
  const page = await host.openExecutionPage(
    host.resolveAskTarget(command.conversationId, command.newConversation),
  );
  await ensureProviderReachable(page);
  return page;
}

async function prepareAskSubmission(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  page: Page,
  timing: AskTiming,
): Promise<{ page: Page; askBaseline: AskBaseline }> {
  if (
    (command.newConversation && !command.conversationId) ||
    (!command.conversationId && !conversationIdFromUrl(safePageUrl(page)))
  ) {
    await resetConversationIdentity(page);
  }
  await waitForInteractionSurface(page, { reloadOnUnavailable: true });
  const askBaseline = await readAskBaseline(page);
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
        Math.max(1, timing.timeoutAt - Date.now()),
      ),
    );
  }
  if (command.mode === "search") {
    await sleep(1_000);
    page = await host.openExecutionPage(safePageUrl(page) ?? page.url());
    await ensureConfiguredSearchMode(page);
  }
  await maybeApplySubmitJitter(command.submitJitterMs, timing.timeoutAt);
  await clickSubmit(page);
  return { page, askBaseline };
}

function createAskRuntimeState(
  command: AskExecutionCommand,
  options: AskExecutionOptions,
  page: Page,
  askBaseline: AskBaseline | null,
): AskRuntimeState {
  return {
    emittedText: options.emittedText,
    nextSeq: options.nextSeq,
    doneStreak: 0,
    knownConversationId:
      options.knownConversationId ??
      command.conversationId ??
      askBaseline?.conversationId ??
      conversationIdFromUrl(safePageUrl(page)) ??
      null,
    knownMessageId: options.knownMessageId ?? null,
    pendingMessageId: null,
    pendingMessageText: "",
    askBaseline: options.askBaseline ?? askBaseline,
    pendingTextReconciliation: false,
    lastProgressAt: Date.now(),
    recoveryAttempts: 0,
    lastStatusAt: 0,
    lastSnapshotSignature: null,
    lastIdentityUpdateSignature: null,
  };
}

async function recoverInitialAskIfNeeded(
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

async function recoverAskSnapshotRead(
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

function throwIfProviderGenerationFailed(snapshot: AskSnapshot): void {
  if (snapshot.failureCode !== PROVIDER_GENERATION_FAILURE_CODE) {
    return;
  }
  noteProviderPressure("generation_failed");
  if (snapshot.retryVisible) {
    noteProviderPressure("retry_visible");
  }
  noteProviderPressureFromText(snapshot.failureMessage);
  throw new AskProviderFailure(
    snapshot.failureMessage ??
      "Something went wrong while generating the response. If this issue persists please contact us through our help center.",
    snapshot.currentUrl,
    snapshot.retryVisible,
  );
}

async function recordAskSnapshotProgress(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  state: AskRuntimeState,
  snapshot: AskSnapshot,
  onChunk: ChunkSink,
  onState: StateSink,
  reason: string,
): Promise<void> {
  rememberAskSnapshot(host, state, snapshot);
  const progressed = await applyAskSnapshot(command, state, snapshot, onChunk);
  await emitAskIdentityUpdate(
    command,
    state,
    snapshot.currentUrl,
    onState,
    reason,
  );
  if (progressed) {
    state.lastProgressAt = Date.now();
  }
}

function updateAskDoneStreak(
  state: AskRuntimeState,
  snapshot: AskSnapshot,
): void {
  if (snapshot.state === "done") {
    state.doneStreak += 1;
  } else {
    state.doneStreak = 0;
  }
}

async function finalizeStableAskIfReady(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  page: Page,
  state: AskRuntimeState,
  snapshot: AskSnapshot,
  onChunk: ChunkSink,
  onState: StateSink,
  timeoutAt: number,
): Promise<{ snapshot: AskSnapshot; result?: AskResultSnapshot }> {
  if (snapshot.state !== "done" || state.doneStreak < STABLE_DONE_POLLS) {
    return { snapshot };
  }
  const stabilized = await stabilizeAskCompletion(
    page,
    state.askBaseline,
    snapshot,
    timeoutAt,
  );
  await recordAskSnapshotProgress(
    host,
    command,
    state,
    stabilized,
    onChunk,
    onState,
    "observed stabilized assistant identity",
  );
  if (stabilized.state === "done") {
    state.pendingTextReconciliation = false;
    return {
      snapshot: stabilized,
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
  state.doneStreak = 0;
  return { snapshot: stabilized };
}

async function recoverStalledAskIfNeeded(
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

async function sleepForAskRecovery(
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

async function sleepForAskPoll(
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

async function ensureProviderReachable(page: Page): Promise<void> {
  const result = await page.evaluate(async (timeoutMs) => {
    const controller = new AbortController();
    const timeoutId = window.setTimeout(() => controller.abort(), timeoutMs);
    try {
      const response = await fetch(new URL("/", window.location.origin), {
        method: "GET",
        credentials: "include",
        cache: "no-store",
        signal: controller.signal,
      });
      if (!response.ok) {
        return {
          ok: false as const,
          timedOut: false,
          reason: `status ${response.status}`,
        };
      }
      return { ok: true as const };
    } catch (error) {
      return {
        ok: false as const,
        timedOut: controller.signal.aborted,
        reason: error instanceof Error ? error.message : String(error),
      };
    } finally {
      window.clearTimeout(timeoutId);
    }
  }, PROVIDER_NETWORK_PREFLIGHT_TIMEOUT_MS);
  if (result.ok) {
    return;
  }
  const suffix = result.timedOut ? "timed out" : "failed";
  const reason = result.reason ? `: ${result.reason}` : "";
  throw new Error(
    `provider/network-unavailable: chatgpt preflight request ${suffix}${reason}`,
  );
}

async function emitAskIdentityUpdate(
  command: AskExecutionCommand,
  state: {
    knownConversationId: string | null;
    knownMessageId: string | null;
    askBaseline: {
      assistantMessageCount: number;
      conversationId: string | null;
      messageId: string | null;
      text: string;
    } | null;
    lastIdentityUpdateSignature: string | null;
  },
  currentUrl: string | null,
  onState: (update: RequestStateSnapshot) => Promise<void> | void,
  reason: string,
): Promise<void> {
  const signature = JSON.stringify([
    state.knownConversationId,
    state.knownMessageId,
    currentUrl,
    state.askBaseline?.conversationId ?? null,
    state.askBaseline?.messageId ?? null,
    state.askBaseline?.assistantMessageCount ?? null,
    state.askBaseline?.text ?? null,
  ]);
  if (state.lastIdentityUpdateSignature === signature) {
    return;
  }
  state.lastIdentityUpdateSignature = signature;
  await onState({
    state: "running",
    reason,
    at: new Date().toISOString(),
    conversation_id:
      state.knownConversationId ?? command.conversationId ?? null,
    message_id: state.knownMessageId,
    current_url: currentUrl,
    streaming_safe: false,
    baseline_conversation_id: state.askBaseline?.conversationId ?? null,
    baseline_message_id: state.askBaseline?.messageId ?? null,
    baseline_assistant_message_count:
      state.askBaseline?.assistantMessageCount ?? null,
    baseline_text: state.askBaseline?.text ?? null,
  });
}

async function maybeApplySubmitJitter(
  submitJitterMs: number | undefined,
  timeoutAt: number,
): Promise<void> {
  const jitterMs = Math.max(0, Math.trunc(submitJitterMs ?? 0));
  if (jitterMs === 0) {
    return;
  }
  const remainingMs = Math.max(0, timeoutAt - Date.now() - 1);
  if (remainingMs === 0) {
    return;
  }
  await sleep(Math.min(jitterMs, remainingMs));
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
