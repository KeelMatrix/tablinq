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

export type { BrowserExecutionHost } from "./types";

export class BrowserExecutionController {
  constructor(private readonly host: BrowserExecutionHost) {}

  async ask(
    command: AskExecutionCommand,
    onChunk: (chunk: StreamChunkSnapshot) => Promise<void> | void,
    onState: (update: RequestStateSnapshot) => Promise<void> | void = () =>
      undefined,
  ): Promise<AskResultSnapshot> {
    return runAsk(this.host, command, onChunk, onState, {
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
    return runResearch(this.host, command, onUpdate, onState, {
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
    return runAsk(this.host, command, onChunk, onState, {
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
    return runResearch(this.host, command, onUpdate, onState, {
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
}
