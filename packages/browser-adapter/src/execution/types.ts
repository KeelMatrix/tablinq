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
  openExecutionPage(
    targetUrl: string,
    forceReload?: boolean,
    options?: {
      failOnProviderConversationMismatch?: boolean;
      providerConversationId?: string | null;
    },
  ): Promise<Page>;
  ensureManagedPage(targetUrl: string, forceReload?: boolean): Promise<Page>;
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
  submittedToProvider: boolean;
  submittedConversationId: string | null;
  pendingMessageId: string | null;
  pendingMessageText: string;
  askBaseline: AskBaseline | null;
  pendingTextReconciliation: boolean;
  lastProgressAt: number;
  lastTextChangeAt: number;
  lastObservedText: string;
  stickyStreamReloaded: boolean;
  stickyStreamFallbackAt: number | null;
  recoveryAttempts: number;
  lastStatusAt: number;
  lastSnapshotSignature: string | null;
  lastProviderActivitySignature: string | null;
  lastProviderActivityAt: number | null;
  lastIdentityUpdateSignature: string | null;
  transientInterruptionThoughtArtifactSignature: string | null;
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
  lastActiveRefreshAt: number;
  recoveryAttempts: number;
  announcedStart: boolean;
  submissionConfirmed: boolean;
  lastStatusAt: number;
  lastSnapshotSignature: string | null;
  lastIdentityUpdateSignature: string | null;
};

export type AskExecutionOptions = {
  submitPrompt: boolean;
  emittedText: string;
  nextSeq: number;
  recoveryReason: string | null;
  knownConversationId?: string | null;
  knownMessageId?: string | null;
  askBaseline?: AskBaseline | null;
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
) => Promise<ResearchResultSnapshot>;
