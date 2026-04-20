import type {
  ResearchExecutionCommand,
  ResearchResultSnapshot,
} from "../contracts";
import {
  applyAskConfiguration,
  beginDeepResearchRun,
  ensureResearchComposerFilled,
  isTransientPageError,
  readLiveResearchSnapshot,
  resetConversationIdentity,
  sleep,
  submitResearch,
  uploadAttachments,
  waitForSubmitReadyAfterUploads,
} from "../pageRuntime";
import {
  computeStallTimeoutMs,
  researchSubmissionStateToSnapshot,
} from "../snapshotState";
import {
  DEEP_RESEARCH_HANDOFF_TIMEOUT_MS,
  INTERACTION_TIMEOUT_MS,
  MAX_RECOVERY_ATTEMPTS,
  RESEARCH_SUBMISSION_START_TIMEOUT_MS,
  RESEARCH_TIMEOUT_MS,
  SNAPSHOT_POLL_INTERVAL_MS,
} from "../timing";

import { finalizeResearch, stabilizeResearchCompletion } from "./finalization";
import { reconcileResearchAfterInterruption } from "./researchRecovery";
import { applyResearchSnapshot, rememberResearchSnapshot } from "./streaming";
import type {
  AskExecutionOptions,
  BrowserExecutionHost,
  ResearchRuntimeState,
  ResearchSelection,
  ResearchUpdateSink,
  StateSink,
} from "./types";

export async function runResearch(
  host: BrowserExecutionHost,
  command: ResearchExecutionCommand,
  onUpdate: ResearchUpdateSink,
  onState: StateSink,
  options: AskExecutionOptions,
  selection: ResearchSelection = {},
): Promise<ResearchResultSnapshot> {
  const executionTimeoutMs = command.timeoutMs ?? RESEARCH_TIMEOUT_MS;
  const stallTimeoutMs = computeStallTimeoutMs(
    command.timeoutMs,
    RESEARCH_TIMEOUT_MS,
  );
  let page = await host.openExecutionPage(
    host.resolveConversationTarget(command.conversationId),
  );
  let submissionState = null;
  let initialSnapshot = null;

  if (options.submitPrompt) {
    if (!command.conversationId) {
      await resetConversationIdentity(page);
    }
    await host.refreshCapabilities(page);
    page = await applyAskConfiguration(
      page,
      {
        jobId: command.jobId,
        prompt: command.query,
        mode: "research",
        modelClass: selection.modelClass,
        effort: selection.effort,
      },
      host.getLastCapabilityReport(),
    );
    await host.refreshCapabilities(page);
    await uploadAttachments(
      page,
      command.files ?? [],
      host.getLastCapabilityReport(),
    );
    await ensureResearchComposerFilled(
      page,
      command.query,
      INTERACTION_TIMEOUT_MS,
    );
    if ((command.files ?? []).length > 0) {
      await waitForSubmitReadyAfterUploads(page);
    }
    submissionState = await submitResearch(
      page,
      command.query,
      RESEARCH_SUBMISSION_START_TIMEOUT_MS,
    );
    initialSnapshot = await beginDeepResearchRun(
      page,
      DEEP_RESEARCH_HANDOFF_TIMEOUT_MS,
    );
  }

  const timeoutAt = Date.now() + executionTimeoutMs;

  const state: ResearchRuntimeState = {
    emittedText: options.emittedText,
    nextSeq: options.nextSeq,
    lastStatus: "starting",
    lastMessage: "starting research",
    doneStreak: 0,
    knownConversationId: command.conversationId ?? null,
    knownMessageId: null,
    lastProgressAt: Date.now(),
    recoveryAttempts: 0,
    announcedStart: false,
    submissionConfirmed: !options.submitPrompt,
    lastStatusAt: 0,
  };

  if (submissionState) {
    rememberResearchSnapshot(
      host,
      state,
      researchSubmissionStateToSnapshot(submissionState),
    );
  }
  if (initialSnapshot) {
    rememberResearchSnapshot(host, state, initialSnapshot);
    const progressed = await applyResearchSnapshot(
      state,
      initialSnapshot,
      onUpdate,
    );
    state.submissionConfirmed = true;
    if (progressed) {
      state.lastProgressAt = Date.now();
    }
  }

  if (!options.submitPrompt) {
    const recovered = await reconcileResearchAfterInterruption(
      host,
      page,
      command,
      state,
      onUpdate,
      onState,
      options.recoveryReason ?? "recovered the interrupted research job",
      timeoutAt,
    );
    page = recovered.page;
    if (recovered.result) {
      return recovered.result;
    }
  }

  while (Date.now() < timeoutAt) {
    let snapshot;
    try {
      snapshot = await readLiveResearchSnapshot(page);
    } catch (error) {
      if (
        isTransientPageError(error) &&
        state.recoveryAttempts < MAX_RECOVERY_ATTEMPTS
      ) {
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
        page = recovered.page;
        if (recovered.result) {
          return recovered.result;
        }
        await sleep(SNAPSHOT_POLL_INTERVAL_MS);
        continue;
      }
      throw error;
    }

    rememberResearchSnapshot(host, state, snapshot);
    const progressed = await applyResearchSnapshot(state, snapshot, onUpdate);
    if (progressed) {
      state.lastProgressAt = Date.now();
    }

    if (snapshot.status === "failed") {
      throw new Error(
        `research job failed: ${snapshot.statusMessage ?? "research failed"}`,
      );
    }

    if (snapshot.status === "done") {
      const stabilized = await stabilizeResearchCompletion(
        page,
        snapshot,
        timeoutAt,
      );
      rememberResearchSnapshot(host, state, stabilized);
      const stabilizedProgress = await applyResearchSnapshot(
        state,
        stabilized,
        onUpdate,
      );
      if (stabilizedProgress) {
        state.lastProgressAt = Date.now();
      }
      if (stabilized.status === "done") {
        return finalizeResearch(host, command, page, stabilized);
      }
    }

    if (
      state.submissionConfirmed &&
      Date.now() - state.lastProgressAt >= stallTimeoutMs &&
      state.recoveryAttempts < MAX_RECOVERY_ATTEMPTS
    ) {
      const recovered = await reconcileResearchAfterInterruption(
        host,
        page,
        command,
        state,
        onUpdate,
        onState,
        "stream stalled while waiting for provider output",
        timeoutAt,
      );
      page = recovered.page;
      if (recovered.result) {
        return recovered.result;
      }
    }

    await sleep(SNAPSHOT_POLL_INTERVAL_MS);
  }

  throw new Error("research job failed: timed out waiting for completion");
}
