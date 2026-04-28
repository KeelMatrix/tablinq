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
import {
  fakeArtifactPage,
  fakeResearchHost,
  fakeResearchSnapshot,
} from "./helpers/researchFinalization";

const harness = installBrowserAdapterHarness();

describe("browser adapter research: recovery and error cases", () => {
  it("fails clearly when /deep-research landing submission never starts and does not emit recovery loops", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
          scenario: "research-deep-landing-submit-fail",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        scenario: "research-deep-landing-submit-fail",
      }),
      headless: true,
    });

    const states: string[] = [];
    await expect(
      adapter.research(
        {
          jobId: "job-research-deep-landing-submit-fail",
          query: "fail deep research landing submission",
          timeoutMs: 2_000,
        },
        () => {},
        (update) => {
          states.push(update.state);
        },
      ),
    ).rejects.toThrow(/deep-research but research never started/i);

    const pageState = await adapter.getManagedPageForTest()!.evaluate(() => ({
      researchLandingVisited: document.body.dataset.researchLandingVisited,
      researchSubmitted: document.body.dataset.researchSubmitted,
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
    }));

    expect(states).toEqual([]);
    expect(pageState.researchLandingVisited).toBe("true");
    expect(pageState.researchSubmitted).toBe("false");
    expect(["button", "keyboard"]).toContain(pageState.lastSubmissionMethod);

    await adapter.shutdown();
  });

  it("fails research jobs with a research-specific error", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
          scenario: "research-failure",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        scenario: "research-failure",
      }),
      headless: true,
    });

    await expect(
      adapter.research(
        {
          jobId: "job-research-failure",
          query: "investigate flaky providers",
          timeoutMs: 5_000,
        },
        () => {},
      ),
    ).rejects.toThrow(/research job failed/i);

    await adapter.shutdown();
  });

  it("fails classic research jobs with a research-specific error", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "research-failure",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        scenario: "research-failure",
      }),
      headless: true,
    });

    await expect(
      adapter.research(
        {
          jobId: "job-research-failure-classic",
          query: "investigate classic flaky providers",
          timeoutMs: 5_000,
        },
        () => {},
      ),
    ).rejects.toThrow(/research job failed/i);

    await adapter.shutdown();
  });

  it("recovers research streaming when the live conversation moves to /c/<id>", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
          scenario: "research-success",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        scenario: "research-success",
      }),
      headless: true,
    });

    const states: string[] = [];
    let closeTriggered = false;
    const result = await adapter.research(
      {
        jobId: "job-research-recover-current-url",
        query: "recover research after browser disconnect",
        timeoutMs: 7_000,
      },
      async (update) => {
        if (
          !closeTriggered &&
          (update.status === "waiting_for_sources" ||
            (update.status === "running" && Boolean(update.chunk_text?.length)))
        ) {
          closeTriggered = true;
          await adapter.getManagedPageForTest()!.context().browser()!.close();
        }
      },
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.report_text).toContain(
      "Research report for: recover research after browser disconnect",
    );
    expect(result.current_url).toContain(`/c/${result.conversation_id}`);
    expect(conversationIdFromUrl(result.current_url)).toBe(
      result.conversation_id,
    );
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
  });

  it("recovers a live research job after the client reconnects to an attached browser", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "research-success",
    });
    const attached = await harness.startAttachableBrowser(targetUrl);

    await withAttachedBrowser(attached.endpoint, async (browser) => {
      const page = browser.contexts()[0]!.pages()[0]!;
      await page.waitForLoadState("domcontentloaded");
      await page.locator("#deep-research-nav").click();
      await page
        .locator("#prompt-textarea")
        .fill("reconnect to a running deep research job");
      await page.locator("[data-testid='send-button']").click();
      await page
        .frameLocator("#deep-research-sandbox")
        .getByRole("button", { name: "Start" })
        .click();
      await page.waitForTimeout(250);
    });

    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const states: string[] = [];
    const updates: string[] = [];
    const result = await adapter.recoverResearch(
      {
        jobId: "job-research-recover-attached-client",
        query: "reconnect to a running deep research job",
        timeoutMs: 7_000,
      },
      { emittedText: "", nextSeq: 0 },
      (update) => {
        updates.push(update.status);
      },
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.report_text).toContain(
      "Research report for: reconnect to a running deep research job",
    );
    expect(updates).toContain("running");
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");
    expect(result.current_url).toContain(`/c/${result.conversation_id}`);

    await adapter.shutdown();
  }, 30_000);

  it("recovers a running deep research job from the active conversation tab when another attached tab is selected", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "research-success",
    });
    const attached = await harness.startAttachableBrowser(targetUrl);

    await withAttachedBrowser(attached.endpoint, async (browser) => {
      const context = browser.contexts()[0]!;
      const rootPage = context.pages()[0]!;
      await rootPage.waitForLoadState("domcontentloaded");
      const researchPage = await context.newPage();
      await researchPage.goto(targetUrl, { waitUntil: "domcontentloaded" });
      await researchPage.locator("#deep-research-nav").click();
      await researchPage
        .locator("#prompt-textarea")
        .fill("recover the active research tab after a client reconnect");
      await researchPage.locator("[data-testid='send-button']").click();
      await researchPage
        .frameLocator("#deep-research-sandbox")
        .getByRole("button", { name: "Start" })
        .click();
      await rootPage.bringToFront();
      await researchPage.waitForTimeout(250);
    });

    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const states: string[] = [];
    const result = await adapter.recoverResearch(
      {
        jobId: "job-research-recover-active-tab",
        query: "recover the active research tab after a client reconnect",
        timeoutMs: 7_000,
      },
      { emittedText: "", nextSeq: 0 },
      () => {},
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.report_text).toContain(
      "Research report for: recover the active research tab after a client reconnect",
    );
    expect(result.current_url).toContain(`/c/${result.conversation_id}`);
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
  }, 30_000);

  it("recovers a running deep research job after the attached research tab reloads", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "research-reload",
    });
    const attached = await harness.startAttachableBrowser(targetUrl);

    await withAttachedBrowser(attached.endpoint, async (browser) => {
      const page = browser.contexts()[0]!.pages()[0]!;
      await page.waitForLoadState("domcontentloaded");
      await page.locator("#deep-research-nav").click();
      await page
        .locator("#prompt-textarea")
        .fill("recover a deep research run after a tab reload");
      await page.locator("[data-testid='send-button']").click();
      await page
        .frameLocator("#deep-research-sandbox")
        .getByRole("button", { name: "Start" })
        .click();
      await page.waitForTimeout(250);
    });

    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const updates: string[] = [];
    const states: string[] = [];
    const result = await adapter.recoverResearch(
      {
        jobId: "job-research-recover-reload",
        query: "recover a deep research run after a tab reload",
        timeoutMs: 7_000,
      },
      { emittedText: "", nextSeq: 0 },
      (update) => {
        updates.push(update.status);
      },
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.report_text).toContain(
      "Research report for: recover a deep research run after a tab reload",
    );
    expect(updates).toContain("running");
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
  }, 30_000);
});
