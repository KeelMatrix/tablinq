import type { Page } from "playwright-core";

import { logRecoveryDecision, safePageUrl } from "../capabilityProbe";
import type {
  RequestStateSnapshot,
  ResearchExecutionCommand,
  ResearchProgressSnapshot,
  ResearchResultSnapshot,
} from "../contracts";
import { beginDeepResearchRun, readLiveResearchSnapshot, sleep } from "../pageRuntime";
import type { ResearchSnapshot } from "../snapshotState";
import { normalizeOptionalString, textRelation } from "../snapshotState";
import {
  DEEP_RESEARCH_HANDOFF_TIMEOUT_MS,
  NAVIGATION_TIMEOUT_MS,
  SNAPSHOT_POLL_INTERVAL_MS,
} from "../timing";
import {
  conversationIdFromUrl,
  researchRecoveryLocationScore,
  uniquePages,
} from "../urls";

import { finalizeResearch, stabilizeResearchCompletion } from "./finalization";
import { applyResearchSnapshot, rememberResearchSnapshot } from "./streaming";
import type { BrowserExecutionHost, ResearchRuntimeState } from "./types";

export async function reconcileResearchAfterInterruption(
  host: BrowserExecutionHost,
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

  let recoveredPage = await host.openExecutionPage(
    host.resolveConversationTarget(
      state.knownConversationId ?? command.conversationId,
    ),
  );
  recoveredPage = await recoverResearchExecutionPage(
    host,
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
    rememberResearchSnapshot(host, state, candidate);
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
    rememberResearchSnapshot(host, state, snapshot);
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

  const progressed = await applyResearchSnapshot(state, snapshot, onUpdate);
  state.submissionConfirmed = true;
  if (progressed) {
    state.doneStreak = 0;
  }
  if (snapshot.status === "done") {
    const stabilized = await stabilizeResearchCompletion(
      recoveredPage,
      snapshot,
      timeoutAt,
    );
    return {
      page: recoveredPage,
      result: await finalizeResearch(host, command, recoveredPage, stabilized),
    };
  }
  return { page: recoveredPage };
}

export async function recoverResearchExecutionPage(
  host: BrowserExecutionHost,
  page: Page,
  knownConversationId: string | null,
  timeoutAt: number,
): Promise<Page> {
  let candidate = await findRecoverableResearchPage(
    host,
    page,
    knownConversationId,
  );
  if (!candidate.isClosed()) {
    await candidate.bringToFront().catch(() => undefined);
  }
  candidate = await reloadResearchPageIfStale(
    candidate,
    knownConversationId,
    timeoutAt,
  );
  if (!host.isManagedPage(candidate)) {
    host.activateRecoveredPage(candidate);
  }
  return candidate;
}

export async function findRecoverableResearchPage(
  host: BrowserExecutionHost,
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
      host.getCurrentTargetUrl(),
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

export async function reloadResearchPageIfStale(
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
