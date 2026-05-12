import type {
  AskResultSnapshot,
  BrowserDiagnosticsSnapshot,
  BrowserStatusSnapshot,
  RequestStateSnapshot,
  ResearchProgressSnapshot,
  ResearchResultSnapshot,
  StreamChunkSnapshot,
} from "../contracts";

export interface ServiceArgs {
  profileDir: string;
  downloadsDir: string;
  defaultTargetUrl: string;
  defaultBrowserPath?: string;
  connectionMode: "attach_first" | "managed_fallback" | "managed_only";
  debugEndpoint?: string;
  extensionRequired?: boolean;
  autoOpenTargetTab?: boolean;
}

export type OpenRequest = {
  type: "open";
  target_url: string;
  browser_path?: string | null;
  headless?: boolean;
  force_reload?: boolean;
};

export type AskRequest = {
  type: "ask";
  job_id: string;
  prompt: string;
  conversation_id?: string | null;
  new_conversation?: boolean;
  timeout_ms?: number;
  submit_jitter_ms?: number;
  mode?: "normal" | "search";
  model_class?: "instant" | "thinking" | null;
  effort?: "none" | "standard" | "extended" | null;
  files?: string[];
};

export type RecoverAskRequest = Omit<AskRequest, "type"> & {
  type: "recover_ask";
  emitted_text?: string | null;
  next_seq?: number | null;
  known_conversation_id?: string | null;
  known_message_id?: string | null;
  baseline_conversation_id?: string | null;
  baseline_message_id?: string | null;
  baseline_assistant_message_count?: number | null;
  baseline_text?: string | null;
};

export type ResearchRequest = {
  type: "research";
  job_id: string;
  query: string;
  conversation_id?: string | null;
  files?: string[];
  output_dir?: string | null;
  timeout_ms?: number;
  submit_jitter_ms?: number;
  emitted_text?: string | null;
  next_seq?: number | null;
};

export type RecoverResearchRequest = Omit<ResearchRequest, "type"> & {
  type: "recover_research";
};

export type DiagnosticsRequest = {
  type: "diagnostics";
  bundle_dir?: string | null;
};

export type ControlRequest =
  | OpenRequest
  | { type: "health" }
  | { type: "close" }
  | { type: "shutdown" };

export type AdapterRequest =
  | OpenRequest
  | AskRequest
  | RecoverAskRequest
  | ResearchRequest
  | RecoverResearchRequest
  | DiagnosticsRequest
  | { type: "health" }
  | { type: "close" }
  | { type: "shutdown" };

export type AdapterFrame =
  | {
      type: "response";
      ok: boolean;
      state: BrowserStatusSnapshot;
      error?: string;
      error_code?: string;
    }
  | { type: "stream_chunk"; chunk: StreamChunkSnapshot }
  | { type: "request_state"; update: RequestStateSnapshot }
  | { type: "research_update"; update: ResearchProgressSnapshot }
  | {
      type: "ask_result";
      ok: boolean;
      state: BrowserStatusSnapshot;
      result?: AskResultSnapshot;
      error?: string;
    }
  | {
      type: "research_result";
      ok: boolean;
      state: BrowserStatusSnapshot;
      result?: ResearchResultSnapshot;
      error?: string;
    }
  | {
      type: "diagnostics";
      ok: boolean;
      state: BrowserStatusSnapshot;
      diagnostics?: BrowserDiagnosticsSnapshot;
      error?: string;
    };
