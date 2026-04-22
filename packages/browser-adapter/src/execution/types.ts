import type { Page } from "playwright-core";

import type {
  AskExecutionCommand,
  AskResultSnapshot,
  CapabilityReportPayload,
  RequestStateSnapshot,
  ResearchExecutionCommand,
  ResearchProgressSnapshot,
  ResearchResultSnapshot,
  StreamChunkSnapshot,
} from "../contracts";
import type { AskBaseline, AskSnapshot } from "../snapshotState";
import type { textRelation } from "../snapshotState";

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

export type AskRuntimeState = {
  emittedText: string;
  nextSeq: number;
  doneStreak: number;
  knownConversationId: string | null;
  knownMessageId: string | null;
  pendingMessageId: string | null;
  pendingMessageText: string;
  askBaseline: AskBaseline | null;
  pendingTextReconciliation: boolean;
  lastProgressAt: number;
  recoveryAttempts: number;
  lastStatusAt: number;
};

export type ResearchRuntimeState = {
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

export type AskExecutionOptions = {
  submitPrompt: boolean;
  emittedText: string;
  nextSeq: number;
  recoveryReason: string | null;
};

export type ResearchSelection = {
  modelClass?: "instant" | "thinking";
  effort?: "none" | "standard" | "extended";
};

export type AskRecoveryResolution = {
  candidate: AskSnapshot;
  relation: ReturnType<typeof textRelation>;
  sameMessageDrift: boolean;
  hardMismatch: boolean;
  canContinue: boolean;
};

export type ChunkSink = (chunk: StreamChunkSnapshot) => Promise<void> | void;

export type StateSink = (update: RequestStateSnapshot) => Promise<void> | void;

export type ResearchUpdateSink = (
  update: ResearchProgressSnapshot,
) => Promise<void> | void;

export type AskRecoveryResult = { page: Page; result?: AskResultSnapshot };
export type ResearchRecoveryResult = {
  page: Page;
  result?: ResearchResultSnapshot;
};

export type AskExecutionFn = (
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  onChunk: ChunkSink,
  onState: StateSink,
  options: AskExecutionOptions,
) => Promise<AskResultSnapshot>;

export type ResearchExecutionFn = (
  host: BrowserExecutionHost,
  command: ResearchExecutionCommand,
  onUpdate: ResearchUpdateSink,
  onState: StateSink,
  options: AskExecutionOptions,
  selection?: ResearchSelection,
) => Promise<ResearchResultSnapshot>;
