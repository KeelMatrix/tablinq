import type {
  AskExecutionCommand,
  BrowserStatusSnapshot,
  ResearchExecutionCommand,
} from "../contracts";
import type { AskBaseline } from "../snapshotState";
import type {
  AskRequest,
  RecoverAskRequest,
  RecoverResearchRequest,
  ResearchRequest,
  ServiceArgs,
} from "./protocol";

export type AskRecoveryState = {
  emittedText: string;
  nextSeq: number;
  knownConversationId: string | null;
  knownMessageId: string | null;
  askBaseline: AskBaseline | null;
};

export type ResearchRecoveryState = {
  emittedText: string;
  nextSeq: number;
};

export function normalizeAskCommand(
  request: AskRequest | RecoverAskRequest,
): AskExecutionCommand {
  const mode = normalizeAskMode(request.mode);
  return {
    jobId: request.job_id,
    prompt: request.prompt,
    conversationId: request.conversation_id,
    newConversation: request.new_conversation,
    timeoutMs: request.timeout_ms,
    submitJitterMs: request.submit_jitter_ms,
    mode,
    modelClass: request.model_class,
    effort: request.effort,
    files: request.files ?? [],
  };
}

export function normalizeAskRecoveryState(
  request: RecoverAskRequest,
): AskRecoveryState {
  return {
    emittedText: request.emitted_text ?? "",
    nextSeq: request.next_seq ?? 0,
    knownConversationId: request.known_conversation_id ?? null,
    knownMessageId: request.known_message_id ?? null,
    askBaseline: normalizeAskBaseline(request),
  };
}

export function normalizeResearchCommand(
  request: ResearchRequest | RecoverResearchRequest,
): ResearchExecutionCommand {
  return {
    jobId: request.job_id,
    query: request.query,
    conversationId: request.conversation_id,
    files: request.files ?? [],
    outputDir: request.output_dir,
    timeoutMs: request.timeout_ms,
    submitJitterMs: request.submit_jitter_ms,
  };
}

export function normalizeResearchRecoveryState(
  request: RecoverResearchRequest,
): ResearchRecoveryState {
  return {
    emittedText: request.emitted_text ?? "",
    nextSeq: request.next_seq ?? 0,
  };
}

export function parseArgs(argv: string[]): ServiceArgs {
  const args = new Map<string, string>();
  for (let index = 0; index < argv.length; index += 2) {
    args.set(argv[index] ?? "", argv[index + 1] ?? "");
  }

  const profileDir = args.get("--profile-dir");
  const downloadsDir = args.get("--downloads-dir");
  const defaultTargetUrl = args.get("--default-target-url");
  const connectionMode = parseConnectionMode(args.get("--connection-mode"));
  if (!profileDir || !downloadsDir || !defaultTargetUrl) {
    throw new Error(
      "service.ts requires --profile-dir, --downloads-dir, --default-target-url, and --connection-mode",
    );
  }

  return {
    profileDir,
    downloadsDir,
    defaultTargetUrl,
    defaultBrowserPath: args.get("--default-browser-path"),
    connectionMode,
    debugEndpoint: args.get("--debug-endpoint") ?? undefined,
    extensionRequired: parseBooleanFlag(args.get("--extension-required"), true),
    autoOpenTargetTab: parseBooleanFlag(
      args.get("--auto-open-target-tab"),
      true,
    ),
  };
}

export function parseBooleanFlag(
  raw: string | undefined,
  fallback: boolean,
): boolean {
  if (!raw) {
    return fallback;
  }
  return raw === "1" || raw.toLowerCase() === "true";
}

export function fallbackStatusSnapshot(
  args: ServiceArgs,
): BrowserStatusSnapshot {
  return {
    running: false,
    browser_kind: process.platform === "win32" ? "msedge" : "chromium",
    profile_dir: args.profileDir,
    downloads_dir: args.downloadsDir,
    target_url: args.defaultTargetUrl,
    current_url: null,
    page_title: null,
    page_ready: false,
    authenticated: false,
    reload_count: 0,
    screenshot_path: null,
    browser_path: args.defaultBrowserPath ?? null,
    extension_connected: false,
    capability_report: null,
    attach_mode: args.connectionMode,
    browser_owner:
      args.connectionMode === "managed_only" ? "managed" : "external",
    cdp_attached: false,
    target_tab_found: false,
    target_tab_id: null,
    target_workspace_key: null,
    debug_endpoint: args.debugEndpoint ?? "http://127.0.0.1:9222",
    auth_source: "unknown",
    extension_required: args.extensionRequired ?? true,
    last_capability_report_at: null,
    action: null,
    browser_process_closed: null,
    reason: null,
  };
}

function parseConnectionMode(
  raw: string | undefined,
): ServiceArgs["connectionMode"] {
  if (
    raw === "attach_first" ||
    raw === "managed_fallback" ||
    raw === "managed_only"
  ) {
    return raw;
  }
  throw new Error(
    "service.ts requires --connection-mode to be one of attach_first, managed_fallback, managed_only",
  );
}

function normalizeAskBaseline(request: RecoverAskRequest): AskBaseline | null {
  if (request.baseline_assistant_message_count == null) {
    return null;
  }
  return {
    assistantMessageCount: request.baseline_assistant_message_count,
    conversationId: request.baseline_conversation_id ?? null,
    messageId: request.baseline_message_id ?? null,
    text: request.baseline_text ?? "",
  };
}

function normalizeAskMode(mode: unknown): AskExecutionCommand["mode"] {
  if (mode == null) {
    return undefined;
  }
  if (mode === "normal" || mode === "search") {
    return mode;
  }
  throw new Error("ask mode unsupported: use the dedicated research command");
}
