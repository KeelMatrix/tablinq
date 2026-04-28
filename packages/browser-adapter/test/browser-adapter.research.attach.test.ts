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

describe("browser adapter research: attach mode", () => {
  it("runs research jobs with progress updates in attach mode", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "research-success",
    });
    const adapter = await harness.createAdapterForMode("attach", targetUrl);
    const fixtureRoot = await harness.createWorkspace("browser");
    const fixture = path.join(fixtureRoot, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    await adapter.open({ targetUrl, headless: true });

    const updates: string[] = [];
    const result = await adapter.research(
      {
        jobId: "job-research-success-attach",
        query: "investigate attach launch risks",
        files: [fixture],
        timeoutMs: 5_000,
      },
      (update) => {
        updates.push(update.status);
      },
    );

    expect(updates).toContain("starting");
    expect(updates).toContain("waiting_for_sources");
    expect(updates).toContain("running");
    expect(result.report_text).toContain(
      "Research report for: investigate attach launch risks",
    );
    expect(result.artifacts).toHaveLength(1);

    await adapter.shutdown();
  }, 30_000);

  it("fails research jobs with a research-specific error in attach mode", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "research-failure",
    });
    const adapter = await harness.createAdapterForMode("attach", targetUrl);
    await adapter.open({ targetUrl, headless: true });

    await expect(
      adapter.research(
        {
          jobId: "job-research-failure-attach",
          query: "investigate attach flaky providers",
          timeoutMs: 5_000,
        },
        () => {},
      ),
    ).rejects.toThrow(/research job failed/i);

    await adapter.shutdown();
  }, 30_000);
});
