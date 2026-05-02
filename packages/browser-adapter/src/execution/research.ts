import type {
  RequestStateSnapshot,
  ResearchExecutionCommand,
  ResearchResultSnapshot,
} from "../contracts";
import type { Page } from "playwright-core";
import {
  beginDeepResearchRun,
  ensureResearchComposerFilled,
  isTransientPageError,
  readLiveResearchSnapshot,
  scrollToBottomIfNeeded,
  sleep,
  submitResearch,
  uploadAttachments,
  waitForDeepResearchLanding,
  waitForSubmitReadyAfterUploads,
} from "../pageRuntime";
import {
  researchSubmissionStateToSnapshot,
  type ResearchSnapshot,
  type ResearchSubmissionStateSnapshot,
} from "../snapshotState";
import {
  ACTIVE_RESEARCH_REFRESH_INTERVAL_MS,
  adaptivePollIntervalMs,
  DEEP_RESEARCH_HANDOFF_TIMEOUT_MS,
  INTERACTION_TIMEOUT_MS,
  MAX_RECOVERY_ATTEMPTS,
  NAVIGATION_TIMEOUT_MS,
  RESEARCH_SUBMISSION_START_TIMEOUT_MS,
  RESEARCH_TIMEOUT_MS,
  noteProviderPressure,
  noteProviderPressureFromText,
} from "../timing";
import { deepResearchPathname } from "../urls";

import { finalizeResearch, stabilizeResearchCompletion } from "./finalization";
import { reconcileResearchAfterInterruption } from "./researchRecovery";
import { applyResearchSnapshot, rememberResearchSnapshot } from "./streaming";
import type {
  AskExecutionOptions,
  BrowserExecutionHost,
  ResearchRecoveryResult,
  ResearchRuntimeState,
  ResearchUpdateSink,
  StateSink,
} from "./types";

export async function runResearch(
  host: BrowserExecutionHost,
  command: ResearchExecutionCommand,
  onUpdate: ResearchUpdateSink,
  onState: StateSink,
  options: AskExecutionOptions,
): Promise<ResearchResultSnapshot> {
  let timing = resolveResearchTiming(command);
  let page = await openResearchExecutionPage(host, command, options);
  await scrollToBottomIfNeeded(page);
  let submissionState: ResearchSubmissionStateSnapshot | null = null;
  let initialSnapshot: ResearchSnapshot | null = null;

  if (options.submitPrompt) {
    const prepared = await prepareResearchSubmission(command, page, timing);
    page = prepared.page;
    submissionState = prepared.submissionState;
    initialSnapshot = prepared.initialSnapshot;
    timing = prepared.timing;
  }

  const state = createResearchRuntimeState(command, options);

  await recordInitialResearchIdentity(
    host,
    state,
    submissionState,
    initialSnapshot,
    onUpdate,
    onState,
  );

  const initialRecovery = await recoverInitialResearchIfNeeded(
    host,
    command,
    page,
    state,
    onUpdate,
    onState,
    options,
    timing.timeoutAt,
  );
  page = initialRecovery.page;
  if (initialRecovery.result) {
    return initialRecovery.result;
  }

  while (Date.now() < timing.timeoutAt) {
    let snapshot: ResearchSnapshot;
    try {
      snapshot = await readLiveResearchSnapshot(page);
    } catch (error) {
      const recovered = await recoverResearchSnapshotRead(
        host,
        command,
        page,
        state,
        onUpdate,
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

    await recordResearchSnapshotProgress(
      host,
      state,
      snapshot,
      onUpdate,
      onState,
      "observed in-flight research identity",
    );

    throwIfResearchFailed(snapshot);

    if (isAwaitingResearchStart(snapshot)) {
      const started = await beginDeepResearchRun(
        page,
        Math.min(
          DEEP_RESEARCH_HANDOFF_TIMEOUT_MS,
          Math.max(1, timing.timeoutAt - Date.now()),
        ),
      );
      if (started) {
        state.submissionConfirmed = true;
        await recordResearchSnapshotProgress(
          host,
          state,
          started,
          onUpdate,
          onState,
          "observed research start confirmation",
        );
      }
      continue;
    }

    const finalized = await finalizeCompletedResearchIfReady(
      host,
      command,
      page,
      state,
      snapshot,
      onUpdate,
      onState,
      timing.timeoutAt,
    );
    if (finalized.result) {
      return finalized.result;
    }

    const activeRefresh = await refreshActiveResearchPageIfNeeded(
      page,
      state,
      snapshot,
      timing.timeoutAt,
    );
    page = activeRefresh.page;
    if (activeRefresh.snapshot) {
      await recordResearchSnapshotProgress(
        host,
        state,
        activeRefresh.snapshot,
        onUpdate,
        onState,
        "observed active research identity after scheduled refresh",
      );
      throwIfResearchFailed(activeRefresh.snapshot);
      const refreshedFinalized = await finalizeCompletedResearchIfReady(
        host,
        command,
        page,
        state,
        activeRefresh.snapshot,
        onUpdate,
        onState,
        timing.timeoutAt,
      );
      if (refreshedFinalized.result) {
        return refreshedFinalized.result;
      }
    }

    await sleepForResearchPoll(state, timing.timeoutAt);
  }

  throw new Error("research job failed: timed out waiting for completion");
}

type ResearchTiming = {
  executionTimeoutMs: number;
  timeoutAt: number;
};

function resolveResearchTiming(
  command: ResearchExecutionCommand,
): ResearchTiming {
  const executionTimeoutMs = command.timeoutMs ?? RESEARCH_TIMEOUT_MS;
  return {
    executionTimeoutMs,
    timeoutAt: Date.now() + executionTimeoutMs,
  };
}

async function openResearchExecutionPage(
  host: BrowserExecutionHost,
  command: ResearchExecutionCommand,
  options: AskExecutionOptions,
): Promise<Page> {
  const targetUrl = options.submitPrompt
    ? resolveDeepResearchSubmissionTarget(host, command)
    : host.resolveConversationTarget(command.conversationId);
  return host.openExecutionPage(targetUrl);
}

function resolveDeepResearchSubmissionTarget(
  host: BrowserExecutionHost,
  command: ResearchExecutionCommand,
): string {
  const baseUrl = host.resolveConversationTarget(command.conversationId);
  try {
    const url = new URL(baseUrl);
    url.pathname = deepResearchPathname(url.pathname);
    return url.toString();
  } catch {
    return baseUrl;
  }
}

async function prepareResearchSubmission(
  command: ResearchExecutionCommand,
  page: Page,
  timing: ResearchTiming,
): Promise<{
  page: Page;
  submissionState: ResearchSubmissionStateSnapshot;
  initialSnapshot: ResearchSnapshot | null;
  timing: ResearchTiming;
}> {
  page =
    (await waitForDeepResearchLanding(page, INTERACTION_TIMEOUT_MS)) ?? page;
  await uploadAttachments(page, command.files ?? [], null);
  await ensureResearchComposerFilled(
    page,
    command.query,
    INTERACTION_TIMEOUT_MS,
  );
  if ((command.files ?? []).length > 0) {
    await waitForSubmitReadyAfterUploads(page);
  }
  await maybeApplySubmitJitter(command.submitJitterMs, timing.timeoutAt);
  const submissionState = await submitResearch(
    page,
    command.query,
    RESEARCH_SUBMISSION_START_TIMEOUT_MS,
  );
  const initialSnapshot = await beginDeepResearchRun(
    page,
    DEEP_RESEARCH_HANDOFF_TIMEOUT_MS,
  );
  return {
    page,
    submissionState,
    initialSnapshot,
    timing: {
      ...timing,
      timeoutAt: Date.now() + timing.executionTimeoutMs,
    },
  };
}

function createResearchRuntimeState(
  command: ResearchExecutionCommand,
  options: AskExecutionOptions,
): ResearchRuntimeState {
  return {
    emittedText: options.emittedText,
    nextSeq: options.nextSeq,
    lastStatus: "starting",
    lastMessage: "starting research",
    doneStreak: 0,
    knownConversationId: command.conversationId ?? null,
    knownMessageId: null,
    lastProgressAt: Date.now(),
    lastActiveRefreshAt: Date.now(),
    recoveryAttempts: 0,
    announcedStart: false,
    submissionConfirmed: !options.submitPrompt,
    lastStatusAt: 0,
    lastSnapshotSignature: null,
    lastIdentityUpdateSignature: null,
  };
}

async function recordInitialResearchIdentity(
  host: BrowserExecutionHost,
  state: ResearchRuntimeState,
  submissionState: ResearchSubmissionStateSnapshot | null,
  initialSnapshot: ResearchSnapshot | null,
  onUpdate: ResearchUpdateSink,
  onState: StateSink,
): Promise<void> {
  if (submissionState) {
    rememberResearchSnapshot(
      host,
      state,
      researchSubmissionStateToSnapshot(submissionState),
    );
    await emitResearchIdentityUpdate(
      state,
      submissionState.currentUrl,
      onState,
      "observed research submission identity",
    );
  }
  if (initialSnapshot) {
    await recordResearchSnapshotProgress(
      host,
      state,
      initialSnapshot,
      onUpdate,
      onState,
      "observed in-flight research identity",
    );
    state.submissionConfirmed = !isAwaitingResearchStart(initialSnapshot);
  }
}

async function recoverInitialResearchIfNeeded(
  host: BrowserExecutionHost,
  command: ResearchExecutionCommand,
  page: Page,
  state: ResearchRuntimeState,
  onUpdate: ResearchUpdateSink,
  onState: StateSink,
  options: AskExecutionOptions,
  timeoutAt: number,
): Promise<ResearchRecoveryResult> {
  if (options.submitPrompt) {
    return { page };
  }
  return reconcileResearchAfterInterruption(
    host,
    page,
    command,
    state,
    onUpdate,
    onState,
    options.recoveryReason ?? "recovered the interrupted research job",
    timeoutAt,
  );
}

async function recoverResearchSnapshotRead(
  host: BrowserExecutionHost,
  command: ResearchExecutionCommand,
  page: Page,
  state: ResearchRuntimeState,
  onUpdate: ResearchUpdateSink,
  onState: StateSink,
  error: unknown,
  timeoutAt: number,
): Promise<ResearchRecoveryResult | null> {
  if (
    !isTransientPageError(error) ||
    state.recoveryAttempts >= MAX_RECOVERY_ATTEMPTS
  ) {
    return null;
  }
  noteProviderPressure("recovery_loop");
  const recovered = await reconcileResearchAfterInterruption(
    host,
    page,
    command,
    state,
    onUpdate,
    onState,
    error instanceof Error ? error.message : String(error),
    timeoutAt,
  );
  if (!recovered.result) {
    await sleepForResearchRecovery(state, timeoutAt);
  }
  return recovered;
}

async function recordResearchSnapshotProgress(
  host: BrowserExecutionHost,
  state: ResearchRuntimeState,
  snapshot: ResearchSnapshot,
  onUpdate: ResearchUpdateSink,
  onState: StateSink,
  reason: string,
): Promise<void> {
  rememberResearchSnapshot(host, state, snapshot);
  const progressed = await applyResearchSnapshot(state, snapshot, onUpdate);
  await emitResearchIdentityUpdate(state, snapshot.currentUrl, onState, reason);
  if (progressed) {
    state.lastProgressAt = Date.now();
  }
}

function throwIfResearchFailed(snapshot: ResearchSnapshot): void {
  if (snapshot.status !== "failed") {
    return;
  }
  noteProviderPressureFromText(snapshot.statusMessage);
  throw new Error(
    `research job failed: ${snapshot.statusMessage ?? "research failed"}`,
  );
}

async function finalizeCompletedResearchIfReady(
  host: BrowserExecutionHost,
  command: ResearchExecutionCommand,
  page: Page,
  state: ResearchRuntimeState,
  snapshot: ResearchSnapshot,
  onUpdate: ResearchUpdateSink,
  onState: StateSink,
  timeoutAt: number,
): Promise<{ result?: ResearchResultSnapshot }> {
  if (snapshot.status !== "done") {
    return {};
  }
  const stabilized = await stabilizeResearchCompletion(
    page,
    snapshot,
    timeoutAt,
  );
  await recordResearchSnapshotProgress(
    host,
    state,
    stabilized,
    onUpdate,
    onState,
    "observed stabilized research identity",
  );
  if (stabilized.status === "done") {
    return {
      result: await finalizeResearch(
        host,
        command,
        page,
        state,
        stabilized,
        onUpdate,
      ),
    };
  }
  return {};
}

export function shouldRefreshActiveResearchPage(
  state: ResearchRuntimeState,
  snapshot: ResearchSnapshot,
  timeoutAt: number,
  now = Date.now(),
  intervalMs = ACTIVE_RESEARCH_REFRESH_INTERVAL_MS,
): boolean {
  if (now >= timeoutAt || !state.submissionConfirmed) {
    return false;
  }
  if (isAwaitingResearchStart(snapshot)) {
    return false;
  }
  if (
    snapshot.status !== "waiting_for_sources" &&
    snapshot.status !== "running"
  ) {
    return false;
  }
  if (snapshot.reportText.length > 0) {
    return false;
  }
  return now - state.lastActiveRefreshAt >= intervalMs;
}

async function refreshActiveResearchPageIfNeeded(
  page: Page,
  state: ResearchRuntimeState,
  snapshot: ResearchSnapshot,
  timeoutAt: number,
): Promise<{ page: Page; snapshot?: ResearchSnapshot }> {
  const now = Date.now();
  if (!shouldRefreshActiveResearchPage(state, snapshot, timeoutAt, now)) {
    return { page };
  }
  state.lastActiveRefreshAt = now;
  await page
    .reload({
      waitUntil: "domcontentloaded",
      timeout: Math.min(NAVIGATION_TIMEOUT_MS, Math.max(1, timeoutAt - now)),
    })
    .catch(() => undefined);
  await scrollToBottomIfNeeded(page).catch(() => undefined);
  const refreshed = await readLiveResearchSnapshot(page).catch(() => null);
  return refreshed ? { page, snapshot: refreshed } : { page };
}

function isAwaitingResearchStart(snapshot: ResearchSnapshot): boolean {
  return snapshot.statusMessage === "awaiting start confirmation";
}

async function sleepForResearchRecovery(
  state: ResearchRuntimeState,
  timeoutAt: number,
): Promise<void> {
  await sleep(
    Math.min(
      adaptivePollIntervalMs("recovery", Date.now() - state.lastProgressAt),
      Math.max(1, timeoutAt - Date.now()),
    ),
  );
}

async function sleepForResearchPoll(
  state: ResearchRuntimeState,
  timeoutAt: number,
): Promise<void> {
  await sleep(
    Math.min(
      adaptivePollIntervalMs("generation", Date.now() - state.lastProgressAt),
      Math.max(1, timeoutAt - Date.now()),
    ),
  );
}

async function emitResearchIdentityUpdate(
  state: {
    knownConversationId: string | null;
    knownMessageId: string | null;
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
  ]);
  if (state.lastIdentityUpdateSignature === signature) {
    return;
  }
  state.lastIdentityUpdateSignature = signature;
  await onState({
    state: "running",
    reason,
    at: new Date().toISOString(),
    conversation_id: state.knownConversationId,
    message_id: state.knownMessageId,
    current_url: currentUrl,
    streaming_safe: false,
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
