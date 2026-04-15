import readline from "node:readline";
import {
  BrowserAdapter,
  type AskExecutionCommand,
  type AskResultSnapshot,
  type BrowserDiagnosticsSnapshot,
  type BrowserStatusSnapshot,
  type RequestStateSnapshot,
  type ResearchExecutionCommand,
  type ResearchProgressSnapshot,
  type ResearchResultSnapshot,
  type StreamChunkSnapshot,
} from "./browserAdapter";

interface ServiceArgs {
  profileDir: string;
  downloadsDir: string;
  defaultTargetUrl: string;
  defaultBrowserPath?: string;
  connectionMode?: "attach_first" | "managed_fallback" | "managed_only";
  debugEndpoint?: string;
  extensionRequired?: boolean;
  autoOpenTargetTab?: boolean;
}

type AdapterRequest =
  | {
      type: "open";
      target_url: string;
      browser_path?: string | null;
      headless?: boolean;
    }
  | {
      type: "ask";
      job_id: string;
      prompt: string;
      conversation_id?: string | null;
      timeout_ms?: number;
      mode?: "normal" | "search" | "research";
      model_class?: "instant" | "thinking" | null;
      effort?: "none" | "standard" | "extended" | null;
      files?: string[];
      emitted_text?: string | null;
      next_seq?: number | null;
    }
  | {
      type: "recover_ask";
      job_id: string;
      prompt: string;
      conversation_id?: string | null;
      timeout_ms?: number;
      mode?: "normal" | "search" | "research";
      model_class?: "instant" | "thinking" | null;
      effort?: "none" | "standard" | "extended" | null;
      files?: string[];
      emitted_text?: string | null;
      next_seq?: number | null;
    }
  | {
      type: "research";
      job_id: string;
      query: string;
      conversation_id?: string | null;
      files?: string[];
      output_dir?: string | null;
      timeout_ms?: number;
      emitted_text?: string | null;
      next_seq?: number | null;
    }
  | {
      type: "recover_research";
      job_id: string;
      query: string;
      conversation_id?: string | null;
      files?: string[];
      output_dir?: string | null;
      timeout_ms?: number;
      emitted_text?: string | null;
      next_seq?: number | null;
    }
  | { type: "diagnostics"; bundle_dir?: string | null }
  | { type: "health" }
  | { type: "close" }
  | { type: "shutdown" };

type AdapterFrame =
  | {
      type: "response";
      ok: boolean;
      state: BrowserStatusSnapshot;
      error?: string;
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

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const adapter = new BrowserAdapter({
    profileDir: args.profileDir,
    downloadsDir: args.downloadsDir,
    defaultTargetUrl: args.defaultTargetUrl,
    defaultBrowserPath: args.defaultBrowserPath,
    connectionMode: args.connectionMode,
    debugEndpoint: args.debugEndpoint,
    extensionRequired: args.extensionRequired,
    autoOpenTargetTab: args.autoOpenTargetTab,
  });

  const rl = readline.createInterface({
    input: process.stdin,
    crlfDelay: Infinity,
  });

  for await (const line of rl) {
    const trimmed = line.trim();
    if (!trimmed) {
      continue;
    }

    let response;
    try {
      const request = JSON.parse(trimmed) as AdapterRequest;
      if (request.type === "ask" || request.type === "recover_ask") {
        const askCommand = {
          jobId: request.job_id,
          prompt: request.prompt,
          conversationId: request.conversation_id,
          timeoutMs: request.timeout_ms,
          mode: request.mode,
          modelClass: request.model_class,
          effort: request.effort,
          files: request.files ?? [],
        } satisfies AskExecutionCommand;
        const finalResult =
          request.type === "recover_ask"
            ? await adapter.recoverAsk(
                askCommand,
                {
                  emittedText: request.emitted_text ?? "",
                  nextSeq: request.next_seq ?? 0,
                },
                (chunk) => {
                  writeFrame({
                    type: "stream_chunk",
                    chunk,
                  });
                },
                (update) => {
                  writeFrame({
                    type: "request_state",
                    update,
                  });
                },
              )
            : await adapter.ask(
                askCommand,
                (chunk) => {
                  writeFrame({
                    type: "stream_chunk",
                    chunk,
                  });
                },
                (update) => {
                  writeFrame({
                    type: "request_state",
                    update,
                  });
                },
              );
        writeFrame({
          type: "ask_result",
          ok: true,
          state: await adapter.snapshot(),
          result: finalResult,
        });
      } else if (
        request.type === "research" ||
        request.type === "recover_research"
      ) {
        const result =
          request.type === "recover_research"
            ? await adapter.recoverResearch(
                {
                  jobId: request.job_id,
                  query: request.query,
                  conversationId: request.conversation_id,
                  files: request.files ?? [],
                  outputDir: request.output_dir,
                  timeoutMs: request.timeout_ms,
                } satisfies ResearchExecutionCommand,
                {
                  emittedText: request.emitted_text ?? "",
                  nextSeq: request.next_seq ?? 0,
                },
                (update) => {
                  writeFrame({
                    type: "research_update",
                    update,
                  });
                },
                (update) => {
                  writeFrame({
                    type: "request_state",
                    update,
                  });
                },
              )
            : await adapter.research(
                {
                  jobId: request.job_id,
                  query: request.query,
                  conversationId: request.conversation_id,
                  files: request.files ?? [],
                  outputDir: request.output_dir,
                  timeoutMs: request.timeout_ms,
                } satisfies ResearchExecutionCommand,
                (update) => {
                  writeFrame({
                    type: "research_update",
                    update,
                  });
                },
                (update) => {
                  writeFrame({
                    type: "request_state",
                    update,
                  });
                },
              );
        writeFrame({
          type: "research_result",
          ok: true,
          state: await adapter.snapshot(),
          result,
        });
      } else if (request.type === "diagnostics") {
        writeFrame({
          type: "diagnostics",
          ok: true,
          state: await adapter.snapshot(),
          diagnostics: await adapter.diagnostics(
            request.bundle_dir ?? undefined,
          ),
        });
      } else {
        const state = await handleRequest(adapter, request);
        response = { type: "response", ok: true, state } satisfies AdapterFrame;
        writeFrame(response);

        if (request.type === "shutdown") {
          break;
        }
      }
    } catch (error) {
      const state = await adapter.health().catch(() => ({
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
        attach_mode: args.connectionMode ?? "managed_only",
        browser_owner:
          (args.connectionMode ?? "managed_only") === "managed_only"
            ? ("managed" as const)
            : ("external" as const),
        cdp_attached: false,
        target_tab_found: false,
        target_tab_id: null,
        debug_endpoint: args.debugEndpoint ?? "http://127.0.0.1:9222",
        auth_source: "unknown" as const,
        extension_required: args.extensionRequired ?? true,
      }));
      response = {
        type: "response",
        ok: false,
        state,
        error: error instanceof Error ? error.message : String(error),
      } satisfies AdapterFrame;
      writeFrame(response);
    }
  }
}

async function handleRequest(
  adapter: BrowserAdapter,
  request: Exclude<
    AdapterRequest,
    | { type: "ask" }
    | { type: "recover_ask" }
    | { type: "research" }
    | { type: "recover_research" }
    | { type: "diagnostics" }
  >,
): Promise<BrowserStatusSnapshot> {
  switch (request.type) {
    case "open":
      return adapter.open({
        targetUrl: request.target_url,
        browserPath: request.browser_path,
        headless: request.headless,
      });
    case "health":
      return adapter.health();
    case "close":
      return adapter.close();
    case "shutdown":
      return adapter.shutdown();
  }
}

function writeFrame(frame: AdapterFrame): void {
  process.stdout.write(`${JSON.stringify(frame)}\n`);
}

function parseArgs(argv: string[]): ServiceArgs {
  const args = new Map<string, string>();
  for (let index = 0; index < argv.length; index += 2) {
    args.set(argv[index] ?? "", argv[index + 1] ?? "");
  }

  const profileDir = args.get("--profile-dir");
  const downloadsDir = args.get("--downloads-dir");
  const defaultTargetUrl = args.get("--default-target-url");
  if (!profileDir || !downloadsDir || !defaultTargetUrl) {
    throw new Error(
      "service.ts requires --profile-dir, --downloads-dir, and --default-target-url",
    );
  }

  return {
    profileDir,
    downloadsDir,
    defaultTargetUrl,
    defaultBrowserPath: args.get("--default-browser-path"),
    connectionMode:
      (args.get("--connection-mode") as ServiceArgs["connectionMode"]) ??
      undefined,
    debugEndpoint: args.get("--debug-endpoint") ?? undefined,
    extensionRequired: parseBooleanFlag(args.get("--extension-required"), true),
    autoOpenTargetTab: parseBooleanFlag(
      args.get("--auto-open-target-tab"),
      true,
    ),
  };
}

function parseBooleanFlag(raw: string | undefined, fallback: boolean): boolean {
  if (!raw) {
    return fallback;
  }
  return raw === "1" || raw.toLowerCase() === "true";
}

void main();
