import readline from "node:readline";
import { BrowserAdapter } from "./browserAdapter";
import { dispatchAdapterRequest } from "./service/dispatch";
import { errorResponseFrame, writeFrame } from "./service/frames";
import { fallbackStatusSnapshot, parseArgs } from "./service/normalization";
import type { AdapterRequest } from "./service/protocol";

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

    try {
      const request = JSON.parse(trimmed) as AdapterRequest;
      const outcome = await dispatchAdapterRequest(
        adapter,
        request,
        writeFrame,
      );
      if (outcome.shutdown) {
        break;
      }
    } catch (error) {
      const state = await adapter
        .health()
        .catch(() => fallbackStatusSnapshot(args));
      writeFrame(errorResponseFrame(state, error));
    }
  }
}

void main();
