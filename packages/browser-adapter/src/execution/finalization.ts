import type { Page } from "playwright-core";

import type {
  AskExecutionCommand,
  AskResultSnapshot,
  ResearchExecutionCommand,
  ResearchResultSnapshot,
} from "../contracts";
import {
  collectArtifacts,
  readAskSnapshot,
  readLiveResearchSnapshot,
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
import {
  SNAPSHOT_POLL_INTERVAL_MS,
  STABLE_DONE_POLLS,
} from "../timing";

import type { BrowserExecutionHost } from "./types";

export async function stabilizeAskCompletion(
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

export async function stabilizeResearchCompletion(
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

export async function finalizeAsk(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  page: Page,
  state: { nextSeq: number },
  snapshot: AskSnapshot,
  onChunk: (chunk: {
    job_id: string;
    conversation_id: string | null;
    message_id: string | null;
    seq: number;
    kind: "done";
    text: string;
  }) => Promise<void> | void,
): Promise<AskResultSnapshot> {
  const artifacts = await collectArtifacts(
    page,
    host.getDownloadsDir(),
    snapshot.conversationId,
    snapshot.messageId,
  );
  await host.ensureManagedPage(
    host.resolveConversationTarget(snapshot.conversationId),
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

export async function finalizeResearch(
  host: BrowserExecutionHost,
  command: ResearchExecutionCommand,
  page: Page,
  snapshot: ResearchSnapshot,
): Promise<ResearchResultSnapshot> {
  const artifacts = await collectArtifacts(
    page,
    host.getDownloadsDir(),
    snapshot.conversationId,
    snapshot.messageId,
  );
  await host.ensureManagedPage(
    host.resolveConversationTarget(snapshot.conversationId),
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
