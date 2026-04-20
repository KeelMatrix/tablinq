import type { Page } from "playwright-core";
import {
  logRecoveryDecision,
  safePageUrl,
} from "./capabilityProbe";
import type {
  AskExecutionCommand,
  AskResultSnapshot,
  CapabilityReportPayload,
  RequestStateSnapshot,
  ResearchExecutionCommand,
  ResearchProgressSnapshot,
  ResearchResultSnapshot,
  StreamChunkSnapshot,
} from "./contracts";
import {
  applyAskConfiguration,
  beginDeepResearchRun,
  clickSubmit,
  collectArtifacts,
  ensureResearchComposerFilled,
  fillComposer,
  isTransientPageError,
  readAskBaseline,
  readAskSnapshot,
  readLiveResearchSnapshot,
  resetConversationIdentity,
  sleep,
  submitResearch,
  uploadAttachments,
  waitForSubmitReadyAfterUploads,
} from "./pageRuntime";
import {
  type AskBaseline,
  type AskSnapshot,
  type ResearchSnapshot,
  type ResearchSubmissionStateSnapshot,
  canFinalizeRecoveredAskSnapshot,
  computeStallTimeoutMs,
  isTransientAskStatusText,
  markdownCompatibleRecoveredText,
  normalizeOptionalString,
  placeholderCompatibleText,
  researchSubmissionStateToSnapshot,
  sameAskCompletionSnapshot,
  sameResearchCompletionSnapshot,
  textRelation,
} from "./snapshotState";
import {
  ASK_STALL_RECOVERY_GRACE_MS,
  ASK_TIMEOUT_MS,
  DEEP_RESEARCH_HANDOFF_TIMEOUT_MS,
  INTERACTION_TIMEOUT_MS,
  MAX_RECOVERY_ATTEMPTS,
  NAVIGATION_TIMEOUT_MS,
  RESEARCH_SUBMISSION_START_TIMEOUT_MS,
  RESEARCH_TIMEOUT_MS,
  SNAPSHOT_POLL_INTERVAL_MS,
  STABLE_DONE_POLLS,
  SUBMIT_READY_AFTER_UPLOAD_TIMEOUT_MS,
} from "./timing";
import {
  conversationIdFromUrl,
  researchRecoveryLocationScore,
  uniquePages,
} from "./urls";

type AskRuntimeState = {
  emittedText: string;
  nextSeq: number;
  doneStreak: number;
  knownConversationId: string | null;
  knownMessageId: string | null;
  askBaseline: AskBaseline | null;
  pendingTextReconciliation: boolean;
  lastProgressAt: number;
  recoveryAttempts: number;
  lastStatusAt: number;
};

type ResearchRuntimeState = {
  emittedText: string;
  nextSeq: number;
  lastStatus: string;
  lastMessage: string;
  doneStreak: number;
  knownConversationId: string | null;
  knownMessageId: string | null;
  lastProgressAt: number;
  recoveryAttempts: number;
  announcedStart: boolean;
  submissionConfirmed: boolean;
  lastStatusAt: number;
};

type AskExecutionOptions = {
  submitPrompt: boolean;
  emittedText: string;
  nextSeq: number;
  recoveryReason: string | null;
};

type ResearchSelection = {
  modelClass?: "instant" | "thinking";
  effort?: "none" | "standard" | "extended";
};

type AskRecoveryResolution = {
  candidate: AskSnapshot;
  relation: ReturnType<typeof textRelation>;
  sameMessageDrift: boolean;
  hardMismatch: boolean;
  canContinue: boolean;
};

export interface BrowserExecutionHost {
  getCurrentTargetUrl(): string;
  setCurrentTargetUrl(url: string): void;
  getLastCapabilityReport(): CapabilityReportPayload | null;
  getDownloadsDir(): string;
  refreshCapabilities(page: Page): Promise<void>;
  openExecutionPage(targetUrl: string): Promise<Page>;
  ensureManagedPage(targetUrl: string): Promise<Page>;
  resolveAskTarget(
    conversationId?: string | null,
    newConversation?: boolean,
  ): string;
  resolveConversationTarget(conversationId?: string | null): string;
  isManagedPage(page: Page): boolean;
  activateRecoveredPage(page: Page): void;
}

export class BrowserExecutionController {
  constructor(private readonly host: BrowserExecutionHost) {}

  async ask(
    command: AskExecutionCommand,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<AskResultSnapshot> {
    return this.runAsk(command, onChunk, onState, {
      submitPrompt: true,
      emittedText: "",
      nextSeq: 0,
      recoveryReason: null,
    });
  }

  async research(
    command: ResearchExecutionCommand,
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<ResearchResultSnapshot> {
    return this.runResearch(command, onUpdate, onState, {
      submitPrompt: true,
      emittedText: "",
      nextSeq: 0,
      recoveryReason: null,
    });
  }

  async recoverAsk(
    command: AskExecutionCommand,
    state: { emittedText: string; nextSeq: number },
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<AskResultSnapshot> {
    return this.runAsk(command, onChunk, onState, {
      submitPrompt: false,
      emittedText: state.emittedText,
      nextSeq: state.nextSeq,
      recoveryReason: "re-attached to the browser session",
    });
  }

  async recoverResearch(
    command: ResearchExecutionCommand,
    state: { emittedText: string; nextSeq: number },
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<ResearchResultSnapshot> {
    return this.runResearch(command, onUpdate, onState, {
      submitPrompt: false,
      emittedText: state.emittedText,
      nextSeq: state.nextSeq,
      recoveryReason: "re-attached to the browser session",
    });
  }

  async reconcileAskAfterInterruptionForTest(
    page: Page,
    command: AskExecutionCommand,
    state: AskRuntimeState,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void,
    reason: string,
    timeoutAt: number,
  ): Promise<{ page: Page; result?: AskResultSnapshot }> {
    return this.reconcileAskAfterInterruption(
      page,
      command,
      state,
      onChunk,
      onState,
      reason,
      timeoutAt,
    );
  }

  private async runAsk(
    command: AskExecutionCommand,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void,
    options: AskExecutionOptions,
  ): Promise<AskResultSnapshot> {
    if (command.mode === "research") {
      return this.runAskAsResearch(command, onChunk, onState, options);
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
    let page = await this.host.openExecutionPage(
      this.host.resolveAskTarget(
        command.conversationId,
        command.newConversation,
      ),
    );
    let askBaseline: AskBaseline | null = null;

    if (options.submitPrompt) {
      if (
        (command.newConversation && !command.conversationId) ||
        (!command.conversationId && !conversationIdFromUrl(safePageUrl(page)))
      ) {
        await resetConversationIdentity(page);
      }
      askBaseline = await readAskBaseline(page);
      await this.host.refreshCapabilities(page);
      if (
        command.modelClass ||
        command.effort ||
        (command.mode && command.mode !== "normal")
      ) {
        page = await applyAskConfiguration(
          page,
          command,
          this.host.getLastCapabilityReport(),
        );
        await this.host.refreshCapabilities(page);
      }
      await uploadAttachments(
        page,
        command.files ?? [],
        this.host.getLastCapabilityReport(),
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
      askBaseline,
      pendingTextReconciliation: false,
      lastProgressAt: Date.now(),
      recoveryAttempts: 0,
      lastStatusAt: 0,
    };

    if (!options.submitPrompt) {
      const recovered = await this.reconcileAskAfterInterruption(
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
      let snapshot: AskSnapshot;
      try {
        snapshot = await readAskSnapshot(page, state.askBaseline);
      } catch (error) {
        if (
          isTransientPageError(error) &&
          state.recoveryAttempts < MAX_RECOVERY_ATTEMPTS
        ) {
          const recovered = await this.reconcileAskAfterInterruption(
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

      this.rememberAskSnapshot(state, snapshot);
      if (
        state.pendingTextReconciliation &&
        textRelation(state.emittedText, snapshot.text) !== "mismatch"
      ) {
        state.pendingTextReconciliation = false;
      }
      const progressed = await this.applyAskSnapshot(
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
        snapshot = await this.stabilizeAskCompletion(
          page,
          state.askBaseline,
          snapshot,
          timeoutAt,
        );
        this.rememberAskSnapshot(state, snapshot);
        const stabilizedProgress = await this.applyAskSnapshot(
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
          return this.finalizeAsk(command, page, state, snapshot, onChunk);
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
        const recovered = await this.reconcileAskAfterInterruption(
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

  private async runAskAsResearch(
    command: AskExecutionCommand,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void,
    options: AskExecutionOptions,
  ): Promise<AskResultSnapshot> {
    let nextSeq = options.nextSeq;
    const result = await this.runResearch(
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

  private async runResearch(
    command: ResearchExecutionCommand,
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void,
    options: AskExecutionOptions,
    selection: ResearchSelection = {},
  ): Promise<ResearchResultSnapshot> {
    const executionTimeoutMs = command.timeoutMs ?? RESEARCH_TIMEOUT_MS;
    const stallTimeoutMs = computeStallTimeoutMs(
      command.timeoutMs,
      RESEARCH_TIMEOUT_MS,
    );
    let page = await this.host.openExecutionPage(
      this.host.resolveConversationTarget(command.conversationId),
    );
    let submissionState: ResearchSubmissionStateSnapshot | null = null;
    let initialSnapshot: ResearchSnapshot | null = null;

    if (options.submitPrompt) {
      if (!command.conversationId) {
        await resetConversationIdentity(page);
      }
      await this.host.refreshCapabilities(page);
      page = await applyAskConfiguration(
        page,
        {
          jobId: command.jobId,
          prompt: command.query,
          mode: "research",
          modelClass: selection.modelClass,
          effort: selection.effort,
        },
        this.host.getLastCapabilityReport(),
      );
      await this.host.refreshCapabilities(page);
      await uploadAttachments(
        page,
        command.files ?? [],
        this.host.getLastCapabilityReport(),
      );
      await ensureResearchComposerFilled(
        page,
        command.query,
        INTERACTION_TIMEOUT_MS,
      );
      if ((command.files ?? []).length > 0) {
        await waitForSubmitReadyAfterUploads(
          page,
          SUBMIT_READY_AFTER_UPLOAD_TIMEOUT_MS,
        );
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
      this.rememberResearchSnapshot(
        state,
        researchSubmissionStateToSnapshot(submissionState),
      );
    }
    if (initialSnapshot) {
      this.rememberResearchSnapshot(state, initialSnapshot);
      const progressed = await this.applyResearchSnapshot(
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
      const recovered = await this.reconcileResearchAfterInterruption(
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
      let snapshot: ResearchSnapshot;
      try {
        snapshot = await readLiveResearchSnapshot(page);
      } catch (error) {
        if (
          isTransientPageError(error) &&
          state.recoveryAttempts < MAX_RECOVERY_ATTEMPTS
        ) {
          const recovered = await this.reconcileResearchAfterInterruption(
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

      this.rememberResearchSnapshot(state, snapshot);
      const progressed = await this.applyResearchSnapshot(
        state,
        snapshot,
        onUpdate,
      );
      if (progressed) {
        state.lastProgressAt = Date.now();
      }

      if (snapshot.status === "failed") {
        throw new Error(
          `research job failed: ${snapshot.statusMessage ?? "research failed"}`,
        );
      }

      if (snapshot.status === "done") {
        const stabilized = await this.stabilizeResearchCompletion(
          page,
          snapshot,
          timeoutAt,
        );
        this.rememberResearchSnapshot(state, stabilized);
        const stabilizedProgress = await this.applyResearchSnapshot(
          state,
          stabilized,
          onUpdate,
        );
        if (stabilizedProgress) {
          state.lastProgressAt = Date.now();
        }
        if (stabilized.status === "done") {
          return this.finalizeResearch(command, page, stabilized);
        }
      }

      if (
        state.submissionConfirmed &&
        Date.now() - state.lastProgressAt >= stallTimeoutMs &&
        state.recoveryAttempts < MAX_RECOVERY_ATTEMPTS
      ) {
        const recovered = await this.reconcileResearchAfterInterruption(
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

  private rememberAskSnapshot(
    state: { knownConversationId: string | null; knownMessageId: string | null },
    snapshot: AskSnapshot,
  ): void {
    if (snapshot.currentUrl) {
      this.host.setCurrentTargetUrl(snapshot.currentUrl);
    }
    const conversationId =
      normalizeOptionalString(snapshot.conversationId) ??
      conversationIdFromUrl(snapshot.currentUrl);
    if (conversationId) {
      state.knownConversationId = conversationId;
    }
    if (snapshot.messageId) {
      state.knownMessageId = snapshot.messageId;
    }
  }

  private rememberResearchSnapshot(
    state: { knownConversationId: string | null; knownMessageId: string | null },
    snapshot: ResearchSnapshot,
  ): void {
    if (snapshot.currentUrl) {
      this.host.setCurrentTargetUrl(snapshot.currentUrl);
    }
    const conversationId =
      normalizeOptionalString(snapshot.conversationId) ??
      conversationIdFromUrl(snapshot.currentUrl);
    if (conversationId) {
      state.knownConversationId = conversationId;
    }
    if (snapshot.messageId) {
      state.knownMessageId = snapshot.messageId;
    }
  }

  private async applyAskSnapshot(
    command: AskExecutionCommand,
    state: { emittedText: string; nextSeq: number },
    snapshot: AskSnapshot,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
  ): Promise<boolean> {
    if (
      snapshot.text.length > state.emittedText.length &&
      snapshot.text.startsWith(state.emittedText)
    ) {
      const delta = snapshot.text.slice(state.emittedText.length);
      state.emittedText = snapshot.text;
      await onChunk({
        job_id: command.jobId,
        conversation_id: snapshot.conversationId,
        message_id: snapshot.messageId,
        seq: state.nextSeq,
        kind: "delta",
        text: delta,
      });
      state.nextSeq += 1;
      return true;
    }
    return false;
  }

  private async applyResearchSnapshot(
    state: {
      emittedText: string;
      nextSeq: number;
      lastStatus: string;
      lastMessage: string;
      announcedStart: boolean;
    },
    snapshot: ResearchSnapshot,
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
  ): Promise<boolean> {
    let progressed = false;
    if (!state.announcedStart) {
      await onUpdate({
        status: "starting",
        message: "starting research",
        conversation_id: snapshot.conversationId,
        message_id: snapshot.messageId,
        current_url: snapshot.currentUrl,
      });
      state.announcedStart = true;
    }
    if (
      snapshot.status !== state.lastStatus ||
      snapshot.statusMessage !== state.lastMessage
    ) {
      state.lastStatus = snapshot.status;
      state.lastMessage = snapshot.statusMessage ?? "";
      if (
        snapshot.status === "waiting_for_sources" ||
        snapshot.status === "running"
      ) {
        await onUpdate({
          status: snapshot.status,
          message: snapshot.statusMessage ?? undefined,
          conversation_id: snapshot.conversationId,
          message_id: snapshot.messageId,
          current_url: snapshot.currentUrl,
        });
      }
    }
    if (
      snapshot.reportText.length > state.emittedText.length &&
      snapshot.reportText.startsWith(state.emittedText)
    ) {
      const delta = snapshot.reportText.slice(state.emittedText.length);
      state.emittedText = snapshot.reportText;
      await onUpdate({
        status: "running",
        message: snapshot.statusMessage ?? undefined,
        conversation_id: snapshot.conversationId,
        message_id: snapshot.messageId,
        current_url: snapshot.currentUrl,
        chunk_kind: "delta",
        chunk_text: delta,
      });
      state.nextSeq += 1;
      progressed = true;
    }
    return progressed;
  }

  private async reconcileAskAfterInterruption(
    page: Page,
    command: AskExecutionCommand,
    state: AskRuntimeState,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void,
    reason: string,
    timeoutAt: number,
  ): Promise<{ page: Page; result?: AskResultSnapshot }> {
    state.recoveryAttempts += 1;
    const interruptedAt = new Date().toISOString();
    if (Date.now() - state.lastStatusAt > 50) {
      await onState({
        state: "interrupted",
        reason,
        at: interruptedAt,
        conversation_id: state.knownConversationId,
        message_id: state.knownMessageId,
        current_url: safePageUrl(page),
        streaming_safe: false,
      });
      state.lastStatusAt = Date.now();
    }

    const currentPageRecovery = await this.tryResumeAskFromPage(
      page,
      command,
      state,
      onChunk,
      onState,
      timeoutAt,
    );
    if (currentPageRecovery) {
      return currentPageRecovery;
    }

    const recoveredPage = await this.host.openExecutionPage(
      this.host.resolveAskTarget(
        state.knownConversationId ??
          command.conversationId ??
          state.askBaseline?.conversationId ??
          conversationIdFromUrl(safePageUrl(page)),
      ),
    );
    if (Date.now() >= timeoutAt) {
      throw new Error(
        "ask timed out while reconciling the interrupted request",
      );
    }

    let snapshot: AskSnapshot | null = null;
    while (Date.now() < timeoutAt) {
      const resolution = this.assessRecoveredAskSnapshot(
        command,
        state,
        await readAskSnapshot(recoveredPage, state.askBaseline),
      );
      if (resolution.canContinue) {
        this.rememberAskSnapshot(state, resolution.candidate);
        if (resolution.relation === "mismatch" && resolution.sameMessageDrift) {
          state.pendingTextReconciliation = true;
        }
        snapshot = resolution.candidate;
        break;
      }
      if (resolution.hardMismatch) {
        throw new Error(
          "interrupted ask could not be reconciled against the live conversation",
        );
      }
      await sleep(SNAPSHOT_POLL_INTERVAL_MS);
    }
    if (!snapshot) {
      throw new Error(
        "interrupted ask could not be reconciled against the live conversation",
      );
    }

    await onState({
      state: "recovered",
      reason:
        snapshot.state === "done"
          ? "recovered the final assistant reply from the live conversation"
          : "recovered the in-flight assistant reply from the live conversation",
      at: new Date().toISOString(),
      conversation_id: snapshot.conversationId,
      message_id: snapshot.messageId,
      current_url: snapshot.currentUrl,
      streaming_safe: snapshot.state === "running",
    });

    const progressed = await this.applyAskSnapshot(
      command,
      state,
      snapshot,
      onChunk,
    );
    if (progressed) {
      state.doneStreak = 0;
    }
    if (snapshot.state === "done") {
      snapshot = await this.stabilizeAskCompletion(
        recoveredPage,
        state.askBaseline,
        snapshot,
        timeoutAt,
      );
      this.rememberAskSnapshot(state, snapshot);
      const stabilizedProgress = await this.applyAskSnapshot(
        command,
        state,
        snapshot,
        onChunk,
      );
      if (stabilizedProgress) {
        state.doneStreak = 0;
      }
    }
    if (snapshot.state === "done") {
      state.doneStreak = STABLE_DONE_POLLS;
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
      return {
        page: recoveredPage,
        result: await this.finalizeAsk(
          command,
          recoveredPage,
          state,
          snapshot,
          onChunk,
        ),
      };
    }
    return { page: recoveredPage };
  }

  private assessRecoveredAskSnapshot(
    command: AskExecutionCommand,
    state: {
      emittedText: string;
      knownConversationId: string | null;
      knownMessageId: string | null;
    },
    candidate: AskSnapshot,
  ): AskRecoveryResolution {
    const relation = textRelation(state.emittedText, candidate.text);
    const relationIsUsable = relation === "same" || relation === "ahead";
    const transientOnlyOutput = isTransientAskStatusText(state.emittedText);
    const commandConversationId = normalizeOptionalString(
      state.knownConversationId ?? command.conversationId,
    );
    const commandMessageId = normalizeOptionalString(state.knownMessageId);
    const snapshotConversationId =
      normalizeOptionalString(candidate.conversationId) ??
      conversationIdFromUrl(candidate.currentUrl);
    const snapshotMessageId = normalizeOptionalString(candidate.messageId);
    const normalizedConversationId =
      snapshotConversationId ?? commandConversationId;
    const conversationMatches =
      !commandConversationId ||
      !snapshotConversationId ||
      commandConversationId === snapshotConversationId;
    const messageMatches =
      commandMessageId !== null &&
      snapshotMessageId !== null &&
      commandMessageId === snapshotMessageId;
    const messageMismatch =
      commandMessageId !== null &&
      snapshotMessageId !== null &&
      commandMessageId !== snapshotMessageId;
    const hasAssistantSignal =
      candidate.hasAssistantMessage &&
      (candidate.messageId !== null || candidate.text.length > 0);
    const sameMessageCompatible =
      relationIsUsable ||
      transientOnlyOutput ||
      placeholderCompatibleText(state.emittedText, candidate.text) ||
      markdownCompatibleRecoveredText(state.emittedText, candidate.text);
    const sameMessageDrift =
      conversationMatches &&
      messageMatches &&
      sameMessageCompatible &&
      (candidate.state === "running" || candidate.state === "done") &&
      hasAssistantSignal;
    const successorMessage =
      conversationMatches &&
      messageMismatch &&
      relationIsUsable &&
      (candidate.state === "running" || candidate.state === "done") &&
      hasAssistantSignal;
    const awaitingLiveState =
      conversationMatches &&
      messageMatches &&
      candidate.state === "idle" &&
      hasAssistantSignal &&
      state.emittedText.length === 0 &&
      candidate.text.length === 0;
    const canContinue =
      (conversationMatches &&
        (relationIsUsable ||
          transientOnlyOutput ||
          sameMessageDrift ||
          successorMessage) &&
        (candidate.state === "running" || candidate.state === "done") &&
        hasAssistantSignal) ||
      awaitingLiveState;
    const hardMismatch =
      !conversationMatches ||
      (messageMatches &&
        candidate.state === "done" &&
        hasAssistantSignal &&
        !sameMessageCompatible &&
        !successorMessage);
    logRecoveryDecision("ask", {
      commandConversationId,
      commandMessageId,
      snapshotConversationId,
      snapshotMessageId,
      snapshotCurrentUrl: candidate.currentUrl,
      normalizedConversationId,
      relation,
      conversationMatches,
      messageMatches,
      messageMismatch,
      hasAssistantSignal,
      snapshotState: candidate.state,
      emittedTextLength: state.emittedText.length,
      observedTextLength: candidate.text.length,
      hasAssistantMessage: candidate.hasAssistantMessage,
      transientOnlyOutput,
      sameMessageCompatible,
      sameMessageDrift,
      successorMessage,
      awaitingLiveState,
      hardMismatch,
      canContinue,
    });
    return {
      candidate,
      relation,
      sameMessageDrift,
      hardMismatch,
      canContinue,
    };
  }

  private async tryResumeAskFromPage(
    page: Page,
    command: AskExecutionCommand,
    state: AskRuntimeState,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void,
    timeoutAt: number,
  ): Promise<{ page: Page; result?: AskResultSnapshot } | null> {
    let resolution: AskRecoveryResolution | null = null;
    try {
      resolution = this.assessRecoveredAskSnapshot(
        command,
        state,
        await readAskSnapshot(page, state.askBaseline),
      );
    } catch {
      return null;
    }
    if (!resolution.canContinue) {
      return null;
    }

    let snapshot = resolution.candidate;
    this.rememberAskSnapshot(state, snapshot);
    if (resolution.relation === "mismatch" && resolution.sameMessageDrift) {
      state.pendingTextReconciliation = true;
    }
    await onState({
      state: "recovered",
      reason:
        snapshot.state === "done"
          ? "recovered the final assistant reply from the live conversation"
          : "recovered the in-flight assistant reply from the live conversation",
      at: new Date().toISOString(),
      conversation_id: snapshot.conversationId,
      message_id: snapshot.messageId,
      current_url: snapshot.currentUrl,
      streaming_safe: snapshot.state === "running",
    });

    const progressed = await this.applyAskSnapshot(
      command,
      state,
      snapshot,
      onChunk,
    );
    if (progressed) {
      state.doneStreak = 0;
    }
    if (snapshot.state === "done") {
      snapshot = await this.stabilizeAskCompletion(
        page,
        state.askBaseline,
        snapshot,
        timeoutAt,
      );
      this.rememberAskSnapshot(state, snapshot);
      const stabilizedProgress = await this.applyAskSnapshot(
        command,
        state,
        snapshot,
        onChunk,
      );
      if (stabilizedProgress) {
        state.doneStreak = 0;
      }
    }
    if (snapshot.state !== "done") {
      return { page };
    }
    state.doneStreak = STABLE_DONE_POLLS;
    if (
      state.pendingTextReconciliation &&
      !canFinalizeRecoveredAskSnapshot(
        state.emittedText,
        state.knownMessageId,
        snapshot,
      )
    ) {
      return null;
    }
    state.pendingTextReconciliation = false;
    return {
      page,
      result: await this.finalizeAsk(command, page, state, snapshot, onChunk),
    };
  }

  private async reconcileResearchAfterInterruption(
    page: Page,
    command: ResearchExecutionCommand,
    state: ResearchRuntimeState,
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void,
    reason: string,
    timeoutAt: number,
  ): Promise<{ page: Page; result?: ResearchResultSnapshot }> {
    state.recoveryAttempts += 1;
    const interruptedAt = new Date().toISOString();
    if (Date.now() - state.lastStatusAt > 50) {
      await onState({
        state: "interrupted",
        reason,
        at: interruptedAt,
        conversation_id: state.knownConversationId,
        message_id: state.knownMessageId,
        current_url: safePageUrl(page),
        streaming_safe: false,
      });
      state.lastStatusAt = Date.now();
    }

    let recoveredPage = await this.host.openExecutionPage(
      this.host.resolveConversationTarget(
        state.knownConversationId ?? command.conversationId,
      ),
    );
    recoveredPage = await this.recoverResearchExecutionPage(
      recoveredPage,
      state.knownConversationId ?? command.conversationId ?? null,
      timeoutAt,
    );
    if (Date.now() >= timeoutAt) {
      throw new Error(
        "research job failed: timed out while reconciling the interrupted request",
      );
    }

    let snapshot: ResearchSnapshot | null = null;
    while (Date.now() < timeoutAt) {
      const candidate = await readLiveResearchSnapshot(recoveredPage);
      this.rememberResearchSnapshot(state, candidate);
      const relation = textRelation(state.emittedText, candidate.reportText);
      const commandConversationId = normalizeOptionalString(
        state.knownConversationId ?? command.conversationId,
      );
      const snapshotConversationId =
        normalizeOptionalString(candidate.conversationId) ??
        conversationIdFromUrl(candidate.currentUrl);
      const normalizedConversationId =
        snapshotConversationId ?? commandConversationId;
      const conversationMatches =
        !commandConversationId ||
        !snapshotConversationId ||
        commandConversationId === snapshotConversationId;
      const messageMatches =
        !state.knownMessageId ||
        !candidate.messageId ||
        state.knownMessageId === candidate.messageId;
      const hasIdentity = Boolean(normalizedConversationId);
      const compatibleText =
        relation !== "mismatch" || (conversationMatches && messageMatches);
      const canContinue =
        conversationMatches &&
        messageMatches &&
        compatibleText &&
        (candidate.status === "waiting_for_sources" ||
          candidate.status === "running" ||
          candidate.status === "done") &&
        (hasIdentity ||
          state.emittedText.length > 0 ||
          candidate.reportText.length > 0);
      const awaitingLiveState =
        hasIdentity &&
        messageMatches &&
        candidate.status === "running" &&
        candidate.reportText.length === 0;
      logRecoveryDecision("research", {
        commandConversationId,
        snapshotConversationId,
        snapshotCurrentUrl: candidate.currentUrl,
        normalizedConversationId,
        relation,
        conversationMatches,
        messageMatches,
        hasIdentity,
        snapshotState: candidate.status,
        emittedTextLength: state.emittedText.length,
        observedTextLength: candidate.reportText.length,
        compatibleText,
        awaitingLiveState,
        canContinue,
      });
      if (canContinue) {
        snapshot = candidate;
        break;
      }
      if (
        !conversationMatches ||
        !messageMatches ||
        (!compatibleText && !awaitingLiveState)
      ) {
        throw new Error(
          "interrupted research job could not be reconciled against the live conversation",
        );
      }
      await sleep(SNAPSHOT_POLL_INTERVAL_MS);
    }
    if (!snapshot) {
      throw new Error(
        "interrupted research job could not be reconciled against the live conversation",
      );
    }

    if (snapshot.statusMessage === "awaiting start confirmation") {
      snapshot =
        (await beginDeepResearchRun(
          recoveredPage,
          Math.min(
            DEEP_RESEARCH_HANDOFF_TIMEOUT_MS,
            Math.max(1, timeoutAt - Date.now()),
          ),
        ).catch(() => snapshot)) ?? snapshot;
      this.rememberResearchSnapshot(state, snapshot);
    }

    await onState({
      state: "recovered",
      reason:
        snapshot.status === "done"
          ? "recovered the final research result from the live conversation"
          : "recovered the in-flight research result from the live conversation",
      at: new Date().toISOString(),
      conversation_id: snapshot.conversationId,
      message_id: snapshot.messageId,
      current_url: snapshot.currentUrl,
      streaming_safe:
        snapshot.status !== "done" && snapshot.status !== "failed",
    });

    const progressed = await this.applyResearchSnapshot(
      state,
      snapshot,
      onUpdate,
    );
    state.submissionConfirmed = true;
    if (progressed) {
      state.doneStreak = 0;
    }
    if (snapshot.status === "done") {
      const stabilized = await this.stabilizeResearchCompletion(
        recoveredPage,
        snapshot,
        timeoutAt,
      );
      return {
        page: recoveredPage,
        result: await this.finalizeResearch(command, recoveredPage, stabilized),
      };
    }
    return { page: recoveredPage };
  }

  private async recoverResearchExecutionPage(
    page: Page,
    knownConversationId: string | null,
    timeoutAt: number,
  ): Promise<Page> {
    let candidate = await this.findRecoverableResearchPage(
      page,
      knownConversationId,
    );
    if (!candidate.isClosed()) {
      await candidate.bringToFront().catch(() => undefined);
    }
    candidate = await this.reloadResearchPageIfStale(
      candidate,
      knownConversationId,
      timeoutAt,
    );
    if (!this.host.isManagedPage(candidate)) {
      this.host.activateRecoveredPage(candidate);
    }
    return candidate;
  }

  private async findRecoverableResearchPage(
    page: Page,
    knownConversationId: string | null,
  ): Promise<Page> {
    const expectedConversationId = normalizeOptionalString(knownConversationId);
    const candidates = uniquePages([
      page,
      ...page.context().pages().filter((candidate) => !candidate.isClosed()),
    ]);

    let best = page;
    let bestScore = Number.NEGATIVE_INFINITY;
    for (const candidate of candidates) {
      const locationScore = researchRecoveryLocationScore(
        safePageUrl(candidate),
        this.host.getCurrentTargetUrl(),
        expectedConversationId,
      );
      if (locationScore < 0) {
        continue;
      }
      const snapshot = await readLiveResearchSnapshot(candidate).catch(() => null);
      const snapshotConversationId =
        normalizeOptionalString(snapshot?.conversationId) ??
        conversationIdFromUrl(snapshot?.currentUrl) ??
        conversationIdFromUrl(safePageUrl(candidate));
      const conversationMatches =
        !expectedConversationId ||
        !snapshotConversationId ||
        snapshotConversationId === expectedConversationId;
      const active =
        snapshot &&
        (snapshot.status === "waiting_for_sources" ||
          snapshot.status === "running" ||
          snapshot.status === "done" ||
          snapshot.statusMessage === "awaiting start confirmation");
      const score =
        locationScore +
        (conversationMatches ? 10 : 0) +
        (active ? 20 : 0) +
        ((snapshot?.reportText.length ?? 0) > 0 ? 5 : 0);
      if (score > bestScore) {
        best = candidate;
        bestScore = score;
      }
    }

    return best;
  }

  private async reloadResearchPageIfStale(
    page: Page,
    knownConversationId: string | null,
    timeoutAt: number,
  ): Promise<Page> {
    const expectedConversationId = normalizeOptionalString(knownConversationId);
    const snapshot = await readLiveResearchSnapshot(page).catch(() => null);
    const snapshotConversationId =
      normalizeOptionalString(snapshot?.conversationId) ??
      conversationIdFromUrl(snapshot?.currentUrl) ??
      conversationIdFromUrl(safePageUrl(page));
    const exactConversation =
      !!expectedConversationId && snapshotConversationId === expectedConversationId;
    const stale =
      !snapshot ||
      ((snapshot.status === "idle" || snapshot.status === "failed") &&
        !snapshot.reportText);
    if (!stale || Date.now() >= timeoutAt) {
      return page;
    }
    if (!exactConversation && expectedConversationId) {
      return page;
    }
    await page
      .reload({
        waitUntil: "domcontentloaded",
        timeout: Math.min(
          NAVIGATION_TIMEOUT_MS,
          Math.max(1, timeoutAt - Date.now()),
        ),
      })
      .catch(() => undefined);
    return page;
  }

  private async stabilizeAskCompletion(
    page: Page,
    baseline: AskBaseline | null,
    snapshot: AskSnapshot,
    timeoutAt: number,
  ): Promise<AskSnapshot> {
    let stableSnapshot = snapshot;
    let stableDonePolls = 0;

    while (Date.now() < timeoutAt && stableDonePolls < STABLE_DONE_POLLS) {
      await sleep(SNAPSHOT_POLL_INTERVAL_MS);
      const candidate = await readAskSnapshot(page, baseline);
      if (candidate.state !== "done") {
        return candidate;
      }
      if (sameAskCompletionSnapshot(stableSnapshot, candidate)) {
        stableDonePolls += 1;
        continue;
      }
      stableSnapshot = candidate;
      stableDonePolls = 0;
    }

    return stableSnapshot;
  }

  private async stabilizeResearchCompletion(
    page: Page,
    snapshot: ResearchSnapshot,
    timeoutAt: number,
  ): Promise<ResearchSnapshot> {
    let stableSnapshot = snapshot;
    let stableDonePolls = 0;

    while (Date.now() < timeoutAt && stableDonePolls < STABLE_DONE_POLLS) {
      await sleep(SNAPSHOT_POLL_INTERVAL_MS);
      const candidate = await readLiveResearchSnapshot(page);
      if (candidate.status !== "done") {
        return candidate;
      }
      if (sameResearchCompletionSnapshot(stableSnapshot, candidate)) {
        stableDonePolls += 1;
        continue;
      }
      stableSnapshot = candidate;
      stableDonePolls = 0;
    }

    return stableSnapshot;
  }

  private async finalizeAsk(
    command: AskExecutionCommand,
    page: Page,
    state: { nextSeq: number },
    snapshot: AskSnapshot,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
  ): Promise<AskResultSnapshot> {
    const artifacts = await collectArtifacts(
      page,
      this.host.getDownloadsDir(),
      snapshot.conversationId,
      snapshot.messageId,
    );
    await this.host.ensureManagedPage(
      this.host.resolveConversationTarget(snapshot.conversationId),
    );
    await onChunk({
      job_id: command.jobId,
      conversation_id: snapshot.conversationId,
      message_id: snapshot.messageId,
      seq: state.nextSeq,
      kind: "done",
      text: "",
    });
    return {
      job_id: command.jobId,
      conversation_id:
        snapshot.conversationId ?? `${command.jobId}-conversation`,
      message_id: snapshot.messageId ?? `${command.jobId}-message`,
      output_text: snapshot.text,
      artifacts,
      completed_at: new Date().toISOString(),
      current_url: snapshot.currentUrl,
    };
  }

  private async finalizeResearch(
    command: ResearchExecutionCommand,
    page: Page,
    snapshot: ResearchSnapshot,
  ): Promise<ResearchResultSnapshot> {
    const artifacts = await collectArtifacts(
      page,
      this.host.getDownloadsDir(),
      snapshot.conversationId,
      snapshot.messageId,
    );
    await this.host.ensureManagedPage(
      this.host.resolveConversationTarget(snapshot.conversationId),
    );
    return {
      job_id: command.jobId,
      conversation_id:
        snapshot.conversationId ?? `${command.jobId}-conversation`,
      message_id: snapshot.messageId ?? `${command.jobId}-message`,
      report_text: snapshot.reportText,
      artifacts,
      completed_at: new Date().toISOString(),
      current_url: snapshot.currentUrl,
      output_path: null,
    };
  }
}
