
import type {
  AskExecutionCommand,
  AskResultSnapshot,
} from "../contracts";

import { runResearch } from "./research";
import type {
  AskExecutionOptions,
  ChunkSink,
  StateSink,
  BrowserExecutionHost,
} from "./types";

export async function runAskAsResearch(
  host: BrowserExecutionHost,
  command: AskExecutionCommand,
  onChunk: ChunkSink,
  onState: StateSink,
  options: AskExecutionOptions,
): Promise<AskResultSnapshot> {
  let nextSeq = options.nextSeq;
  const result = await runResearch(
    host,
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
