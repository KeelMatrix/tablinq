import type { Page } from "playwright-core";

import type {
  AskExecutionCommand,
  AskResultSnapshot,
  RequestStateSnapshot,
  ResearchExecutionCommand,
  ResearchProgressSnapshot,
  ResearchResultSnapshot,
  StreamChunkSnapshot,
} from "../contracts";

import { reconcileAskAfterInterruption } from "./askRecovery";
import { runAsk } from "./ask";
import { runResearch } from "./research";
import type { AskRuntimeState, BrowserExecutionHost } from "./types";
import type { AskBaseline } from "../snapshotState";
import { workspaceKeyFromUrl } from "../urls";

export type { BrowserExecutionHost } from "./types";

export class BrowserExecutionController {
  private readonly activeInteractiveTargets = new Map<string, string>();

  constructor(private readonly host: BrowserExecutionHost) {}

  async ask(
    command: AskExecutionCommand,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<AskResultSnapshot> {
    return this.runWithSingleFlight(
      this.interactiveTargetKey(
        this.host.resolveAskTarget(
          command.conversationId,
          command.newConversation,
        ),
      ),
      "ask",
      () =>
        runAsk(this.host, command, onChunk, onState, {
          submitPrompt: true,
          emittedText: "",
          nextSeq: 0,
          recoveryReason: null,
        }),
    );
  }

  async research(
    command: ResearchExecutionCommand,
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<ResearchResultSnapshot> {
    return this.runWithSingleFlight(
      this.interactiveTargetKey(
        this.host.resolveConversationTarget(command.conversationId),
      ),
      "research",
      () =>
        runResearch(this.host, command, onUpdate, onState, {
          submitPrompt: true,
          emittedText: "",
          nextSeq: 0,
          recoveryReason: null,
        }),
    );
  }

  async recoverAsk(
    command: AskExecutionCommand,
    state: {
      emittedText: string;
      nextSeq: number;
      knownConversationId?: string | null;
      knownMessageId?: string | null;
      askBaseline?: AskBaseline | null;
    },
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<AskResultSnapshot> {
    return this.runWithSingleFlight(
      this.interactiveTargetKey(
        this.host.resolveAskTarget(
          command.conversationId,
          command.newConversation,
        ),
      ),
      "recover ask",
      () =>
        runAsk(this.host, command, onChunk, onState, {
          submitPrompt: false,
          emittedText: state.emittedText,
          nextSeq: state.nextSeq,
          recoveryReason: "re-attached to the browser session",
          knownConversationId: state.knownConversationId,
          knownMessageId: state.knownMessageId,
          askBaseline: state.askBaseline,
        }),
    );
  }

  async recoverResearch(
    command: ResearchExecutionCommand,
    state: { emittedText: string; nextSeq: number },
    onUpdate: (update: ResearchProgressSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<ResearchResultSnapshot> {
    return this.runWithSingleFlight(
      this.interactiveTargetKey(
        this.host.resolveConversationTarget(command.conversationId),
      ),
      "recover research",
      () =>
        runResearch(this.host, command, onUpdate, onState, {
          submitPrompt: false,
          emittedText: state.emittedText,
          nextSeq: state.nextSeq,
          recoveryReason: "re-attached to the browser session",
        }),
    );
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
    return reconcileAskAfterInterruption(
      this.host,
      page,
      command,
      state,
      onChunk,
      onState,
      reason,
      timeoutAt,
    );
  }

  private interactiveTargetKey(targetUrl: string): string {
    return workspaceKeyFromUrl(targetUrl);
  }

  private async runWithSingleFlight<T>(
    targetKey: string,
    action: string,
    run: () => Promise<T>,
  ): Promise<T> {
    const activeAction = this.activeInteractiveTargets.get(targetKey);
    if (activeAction) {
      throw new Error(
        `browser surface busy: ${activeAction} is already running for workspace ${targetKey}`,
      );
    }

    this.activeInteractiveTargets.set(targetKey, action);
    try {
      return await run();
    } finally {
      if (this.activeInteractiveTargets.get(targetKey) === action) {
        this.activeInteractiveTargets.delete(targetKey);
      }
    }
  }
}
