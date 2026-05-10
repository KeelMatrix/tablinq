import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { JSDOM } from "jsdom";
import type { Page } from "playwright-core";
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
import { applyResearchSnapshot } from "../src/execution/streaming";
import type { ResearchSnapshot } from "../src/snapshotState";
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
import {
  fakeArtifactPage,
  fakeResearchHost,
  fakeResearchSnapshot,
} from "./helpers/researchFinalization";

const harness = installBrowserAdapterHarness();

async function installResearchStartupInteractionGuard(
  page: Page,
): Promise<void> {
  const guardScript = () => {
    const global = window as Window & {
      __tablinqResearchStartupGuardInstalled?: boolean;
    };
    if (global.__tablinqResearchStartupGuardInstalled) {
      return;
    }
    global.__tablinqResearchStartupGuardInstalled = true;
    const increment = (key: string) => {
      localStorage.setItem(
        key,
        String(Number(localStorage.getItem(key) ?? "0") + 1),
      );
    };
    document.addEventListener(
      "click",
      (event) => {
        const target = event.target;
        if (!(target instanceof Element)) {
          return;
        }
        if (target.closest("#deep-research-nav")) {
          increment("tablinqDeepResearchNavClicks");
        }
        if (
          target.closest(
            "nav, [data-testid='create-new-chat-button'], [aria-label*='sidebar' i]",
          )
        ) {
          increment("tablinqSidebarInteractions");
        }
      },
      true,
    );
    document.addEventListener(
      "keydown",
      (event) => {
        if (
          event.ctrlKey &&
          event.shiftKey &&
          event.key.toLowerCase() === "o"
        ) {
          increment("tablinqNewChatShortcuts");
        }
      },
      true,
    );
  };

  await page.evaluate(() => {
    for (const key of [
      "tablinqDeepResearchNavClicks",
      "tablinqSidebarInteractions",
      "tablinqNewChatShortcuts",
    ]) {
      localStorage.setItem(key, "0");
    }
  });
  await page.addInitScript(guardScript);
  await page.evaluate(guardScript);
}

describe("browser adapter research: attach mode", () => {
  it("emits a waiting-for-sources handoff before the first running snapshot", async () => {
    const updates: string[] = [];
    const state = {
      emittedText: "",
      nextSeq: 0,
      lastStatus: "starting",
      lastMessage: "starting research",
      announcedStart: false,
      lastSnapshotSignature: null,
    };
    const snapshot: ResearchSnapshot = {
      status: "running",
      statusMessage: "Drafting report",
      reportText: "",
      conversationId: "conv-research-attach",
      messageId: "msg-research-attach",
      currentUrl: "https://chatgpt.test/c/conv-research-attach",
    };

    await applyResearchSnapshot(state, snapshot, (update) => {
      updates.push(update.status);
    });

    expect(updates).toEqual(["starting", "waiting_for_sources", "running"]);
  });

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

  it("enters Deep Research in attach mode without sidebar or new-chat controls", async () => {
    const server = await harness.startServer();
    const target = new URL(
      withHarnessUrl(server.url, {
        variant: "live",
        scenario: "research-success",
      }),
    );
    target.pathname = "/c/existing-research-thread";
    const adapter = await harness.createAdapterForMode(
      "attach",
      target.toString(),
    );
    await adapter.open({ targetUrl: target.toString(), headless: true });
    const page = adapter.getManagedPageForTest()!;
    await installResearchStartupInteractionGuard(page);

    const result = await adapter.research(
      {
        jobId: "job-research-attach-no-sidebar",
        query: "check attach research without sidebar activation",
        timeoutMs: 8_000,
      },
      () => {},
    );
    const pageState = await adapter.getManagedPageForTest()!.evaluate(() => ({
      navClicks: localStorage.getItem("tablinqDeepResearchNavClicks"),
      sidebarInteractions: localStorage.getItem("tablinqSidebarInteractions"),
      newChatShortcuts: localStorage.getItem("tablinqNewChatShortcuts"),
      newChatClicks: document.body.dataset.newChatClicks ?? "0",
      researchLandingVisited: document.body.dataset.researchLandingVisited,
    }));

    expect(result.report_text).toContain(
      "Research report for: check attach research without sidebar activation",
    );
    expect(pageState).toMatchObject({
      navClicks: "0",
      sidebarInteractions: "0",
      newChatShortcuts: "0",
      newChatClicks: "0",
      researchLandingVisited: "true",
    });

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
