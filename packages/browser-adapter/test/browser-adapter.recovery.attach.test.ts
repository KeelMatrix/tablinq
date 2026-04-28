import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { JSDOM } from "jsdom";
import {
  adaptivePollIntervalMs,
  BrowserAdapter,
  chooseUniqueArtifactPath,
  conversationIdFromUrl,
  deepResearchStartDelayMs,
  detectBrowserExecutable,
  interactionJitterMs,
  MAX_UPLOAD_FILE_BYTES,
  resolveConversationUrl,
  resolveBrowserLaunchConfig,
  uploadAckTimeoutMsForFiles,
  workspaceKeyFromUrl,
} from "../src/browserAdapter";
import { markdownCompatibleRecoveredText } from "../src/snapshotState";
import {
  createIsolatedTempDir,
  listAttachedPages,
  withAttachedBrowser,
  withHarnessUrl,
} from "./browser-adapter.integration.helpers";
import { openManagedHarnessAdapter } from "./helpers/browserFlow";
import { installBrowserAdapterHarness } from "./helpers/browserAdapterHarness";
import { maybeCloseManagedBrowserOnRunningIdentity } from "./helpers/recoveryFlow";

const harness = installBrowserAdapterHarness();

describe("browser adapter recovery: attach recovery", () => {
  it("continues ask streaming after a page reload in attach mode", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, { scenario: "reload" });
    const adapter = await harness.createAdapterForMode("attach", targetUrl);
    await adapter.open({ targetUrl, headless: true });

    const states: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-reload-attach",
        prompt: "survive attach reloads",
        timeoutMs: 5000,
      },
      () => {},
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.output_text).toBe(
      "Fake response for: survive attach reloads",
    );
    expect((await adapter.health()).reload_count).toBeGreaterThan(0);

    await adapter.shutdown();
  });

  it("recovers ask streaming after a browser disconnect in attach mode", async () => {
    const server = await harness.startServer();
    const adapter = await harness.createAdapterForMode("attach", server.url);
    await adapter.open({ targetUrl: server.url, headless: true });

    const states: string[] = [];
    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-browser-disconnect-attach",
        prompt: "recover after attach disconnect",
        timeoutMs: 7000,
      },
      () => {},
      async (update) => {
        states.push(update.state);
        closeTriggered = await maybeCloseManagedBrowserOnRunningIdentity(
          adapter,
          update,
          closeTriggered,
        );
      },
    );

    const result = await request;
    expect(result.output_text).toBe(
      "Fake response for: recover after attach disconnect",
    );
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
  });
});
