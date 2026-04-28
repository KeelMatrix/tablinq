import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { JSDOM } from "jsdom";
import {
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
import { finalizeResearch } from "../src/execution/finalization";
import {
  createIsolatedTempDir,
  listAttachedPages,
  withAttachedBrowser,
  withHarnessUrl,
} from "./browser-adapter.integration.helpers";
import {
  captureResearchStatuses,
  expectResearchHandoffStatuses,
  openManagedHarnessAdapter,
} from "./helpers/browserFlow";
import { installBrowserAdapterHarness } from "./helpers/browserAdapterHarness";
import { fakeArtifactPage, fakeResearchHost, fakeResearchSnapshot } from "./helpers/researchFinalization";

const harness = installBrowserAdapterHarness();

describe("browser adapter research: result extraction", () => {
  it("includes uploaded file names in classic research output", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "research-success",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        scenario: "research-success",
      }),
      headless: true,
    });

    const result = await adapter.research(
      {
        jobId: "job-research-success-classic",
        query: "summarize the upload",
        files: [fixture],
        timeoutMs: 5_000,
      },
      () => {},
    );

    expect(result.report_text).toContain("brief.txt");

    await adapter.shutdown();
  });
});
