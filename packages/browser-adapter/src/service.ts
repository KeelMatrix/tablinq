import readline from "node:readline";
import { BrowserAdapter } from "./browserAdapter";

interface ServiceArgs {
  profileDir: string;
  downloadsDir: string;
  defaultTargetUrl: string;
  defaultBrowserPath?: string;
}

type AdapterRequest =
  | {
      type: "open";
      target_url: string;
      browser_path?: string | null;
      headless?: boolean;
    }
  | { type: "health" }
  | { type: "close" }
  | { type: "shutdown" };

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const adapter = new BrowserAdapter({
    profileDir: args.profileDir,
    downloadsDir: args.downloadsDir,
    defaultTargetUrl: args.defaultTargetUrl,
    defaultBrowserPath: args.defaultBrowserPath,
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
      const state = await handleRequest(adapter, request);
      response = { ok: true, state };
      process.stdout.write(`${JSON.stringify(response)}\n`);

      if (request.type === "shutdown") {
        break;
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
      }));
      response = {
        ok: false,
        state,
        error: error instanceof Error ? error.message : String(error),
      };
      process.stdout.write(`${JSON.stringify(response)}\n`);
    }
  }
}

async function handleRequest(adapter: BrowserAdapter, request: AdapterRequest) {
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
  };
}

void main();
