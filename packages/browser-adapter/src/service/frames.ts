import type {
  BrowserStatusSnapshot,
  RequestStateSnapshot,
  ResearchProgressSnapshot,
  StreamChunkSnapshot,
} from "../contracts";
import type { AdapterFrame } from "./protocol";

export type FrameWriter = (frame: AdapterFrame) => void;

export function streamChunkFrame(chunk: StreamChunkSnapshot): AdapterFrame {
  return {
    type: "stream_chunk",
    chunk,
  };
}

export function requestStateFrame(update: RequestStateSnapshot): AdapterFrame {
  return {
    type: "request_state",
    update,
  };
}

export function researchUpdateFrame(
  update: ResearchProgressSnapshot,
): AdapterFrame {
  return {
    type: "research_update",
    update,
  };
}

export function errorResponseFrame(
  state: BrowserStatusSnapshot,
  error: unknown,
): AdapterFrame {
  return {
    type: "response",
    ok: false,
    state,
    error: error instanceof Error ? error.message : String(error),
    error_code:
      error &&
      typeof error === "object" &&
      "code" in error &&
      typeof error.code === "string"
        ? error.code
        : undefined,
  };
}

export function encodeFrame(frame: AdapterFrame): string {
  return `${JSON.stringify(frame)}\n`;
}

export function writeFrame(
  frame: AdapterFrame,
  output: NodeJS.WritableStream = process.stdout,
): void {
  output.write(encodeFrame(frame));
}
