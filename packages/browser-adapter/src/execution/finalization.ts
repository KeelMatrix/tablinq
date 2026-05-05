import type { Page } from "playwright-core";

import type {
  AskExecutionCommand,
  AskResultSnapshot,
  ResearchProgressSnapshot,
  ResearchExecutionCommand,
  ResearchResultSnapshot,
  StreamChunkSnapshot,
} from "../contracts";
import {
  collectArtifacts,
  materializeAskCodePreviewSources,
  readArtifactStabilitySignature,
  readAskSnapshot,
  readLiveResearchSnapshot,
  scrollToBottomIfNeeded,
  sleep,
} from "../pageRuntime";
import type {
  AskBaseline,
  AskSnapshot,
  ResearchSnapshot,
} from "../snapshotState";
import {
  sameAskCompletionSnapshot,
  sameResearchCompletionSnapshot,
} from "../snapshotState";
import { SNAPSHOT_POLL_INTERVAL_MS, STABLE_DONE_POLLS } from "../timing";

import type { BrowserExecutionHost } from "./types";

export async function stabilizeAskCompletion(
  page: Page,
  baseline: AskBaseline | null,
  snapshot: AskSnapshot,
  timeoutAt: number,
): Promise<AskSnapshot> {
  await materializeAskCodePreviewSources(page, baseline).catch(() => 0);
  let stableSnapshot =
    (await readAskSnapshot(page, baseline).catch(() => snapshot)) ?? snapshot;
  let stableArtifactSignature = await artifactSignatureForSnapshot(
    page,
    stableSnapshot.messageId,
  );
  let stableDonePolls = 0;

  while (Date.now() < timeoutAt && stableDonePolls < STABLE_DONE_POLLS) {
    await sleep(SNAPSHOT_POLL_INTERVAL_MS);
    await materializeAskCodePreviewSources(page, baseline).catch(() => 0);
    const candidate = await readAskSnapshot(page, baseline);
    if (candidate.state !== "done") {
      return candidate;
    }
    const artifactSignature = await artifactSignatureForSnapshot(
      page,
      candidate.messageId,
    );
    if (candidate.hasTransientWidgets) {
      stableSnapshot = candidate;
      stableArtifactSignature = artifactSignature;
      stableDonePolls = 0;
      continue;
    }
    if (
      sameAskCompletionSnapshot(stableSnapshot, candidate) &&
      artifactSignature === stableArtifactSignature
    ) {
      stableDonePolls += 1;
      continue;
    }
    stableSnapshot = candidate;
    stableArtifactSignature = artifactSignature;
    stableDonePolls = 0;
  }

  return stableSnapshot;
}

export async function stabilizeResearchCompletion(
  page: Page,
  snapshot: ResearchSnapshot,
  timeoutAt: number,
): Promise<ResearchSnapshot> {
  let stableSnapshot = snapshot;
  let stableArtifactSignature = await artifactSignatureForSnapshot(
    page,
    snapshot.messageId,
  );
  let stableDonePolls = 0;

  while (Date.now() < timeoutAt && stableDonePolls < STABLE_DONE_POLLS) {
    await sleep(SNAPSHOT_POLL_INTERVAL_MS);
    const candidate = await readLiveResearchSnapshot(page);
    if (candidate.status !== "done") {
      return candidate;
    }
    const artifactSignature = await artifactSignatureForSnapshot(
      page,
      candidate.messageId,
    );
    if (
      sameResearchCompletionSnapshot(stableSnapshot, candidate) &&
      artifactSignature === stableArtifactSignature
    ) {
      stableDonePolls += 1;
      continue;
    }
    stableSnapshot = candidate;
    stableArtifactSignature = artifactSignature;
    stableDonePolls = 0;
  }

  return stableSnapshot;
}

async function artifactSignatureForSnapshot(
  page: Page,
  messageId: string | null,
): Promise<string> {
  return readArtifactStabilitySignature(page, messageId).catch(() => "");
}

export async function finalizeAsk(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  page: Page,
  state: { emittedText?: string; nextSeq: number },
  snapshot: AskSnapshot,
  onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
): Promise<AskResultSnapshot> {
  const artifacts = await collectArtifacts(
    page,
    host.getDownloadsDir(),
    snapshot.conversationId,
    snapshot.messageId,
  );
  await scrollToBottomIfNeeded(page);
  await host.ensureManagedPage(
    host.resolveConversationTarget(snapshot.conversationId),
  );
  const emittedText = state.emittedText ?? "";
  if (snapshot.text && emittedText !== snapshot.text) {
    await onChunk({
      job_id: command.jobId,
      conversation_id: snapshot.conversationId,
      message_id: snapshot.messageId,
      seq: state.nextSeq,
      kind: "delta",
      text: snapshot.text,
    });
    state.emittedText = snapshot.text;
    state.nextSeq += 1;
  }
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
    conversation_id: snapshot.conversationId ?? `${command.jobId}-conversation`,
    message_id: snapshot.messageId ?? `${command.jobId}-message`,
    output_text: snapshot.text,
    artifacts,
    completed_at: new Date().toISOString(),
    current_url: snapshot.currentUrl,
  };
}

export async function finalizeResearch(
  host: BrowserExecutionHost,
  command: ResearchExecutionCommand,
  page: Page,
  state: { emittedText?: string; nextSeq: number },
  snapshot: ResearchSnapshot,
  onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
): Promise<ResearchResultSnapshot> {
  const artifacts = await collectArtifacts(
    page,
    host.getDownloadsDir(),
    snapshot.conversationId,
    snapshot.messageId,
  );
  await scrollToBottomIfNeeded(page);
  await host.ensureManagedPage(
    host.resolveConversationTarget(snapshot.conversationId),
  );
  const emittedText = state.emittedText ?? "";
  if (snapshot.reportText && emittedText !== snapshot.reportText) {
    await onUpdate({
      status: "running",
      message: snapshot.statusMessage ?? undefined,
      conversation_id: snapshot.conversationId,
      message_id: snapshot.messageId,
      current_url: snapshot.currentUrl,
      chunk_kind: "delta",
      chunk_text: snapshot.reportText,
    });
    state.emittedText = snapshot.reportText;
    state.nextSeq += 1;
  }
  return {
    job_id: command.jobId,
    conversation_id: snapshot.conversationId ?? `${command.jobId}-conversation`,
    message_id: snapshot.messageId ?? `${command.jobId}-message`,
    report_text: snapshot.reportText,
    artifacts,
    completed_at: new Date().toISOString(),
    current_url: snapshot.currentUrl,
    output_path: null,
  };
}
