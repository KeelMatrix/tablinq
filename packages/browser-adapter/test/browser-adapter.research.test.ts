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
import {
  createBrowserIntegrationHarness,
  createIsolatedTempDir,
  listAttachedPages,
  withAttachedBrowser,
  withHarnessUrl,
} from "./browser-adapter.integration.helpers";

let harness: ReturnType<typeof createBrowserIntegrationHarness>;

beforeEach(() => {
  harness = createBrowserIntegrationHarness();
});

afterEach(async () => {
  await harness.cleanup();
});

describe("browser adapter", () => {
  it("runs research jobs with progress updates and persisted artifacts", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixtureDir = path.join(root, "fixtures");
    const firstFile = path.join(fixtureDir, "brief.txt");
    const secondFile = path.join(fixtureDir, "notes.csv");
    await mkdir(fixtureDir, { recursive: true });
    await writeFile(firstFile, "brief body", "utf8");
    await writeFile(secondFile, "col\nvalue", "utf8");
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

    const updates: string[] = [];
    const result = await adapter.research(
      {
        jobId: "job-research-success",
        query: "investigate launch risks",
        files: [firstFile, secondFile],
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
      "Research report for: investigate launch risks",
    );
    expect(result.artifacts).toHaveLength(1);
    const artifactBody = await readFile(
      result.artifacts[0]!.storage_path,
      "utf8",
    );
    expect(artifactBody).toContain("Uploads: brief.txt, notes.csv");

    await adapter.shutdown();
  });

  it("does not finalize Deep Research early when a post-start shell shows provisional text", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
          scenario: "research-starting-shell",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        scenario: "research-starting-shell",
      }),
      headless: true,
    });

    const updates: string[] = [];
    const result = await adapter.research(
      {
        jobId: "job-research-starting-shell",
        query:
          "analyze browser-backed provider tradeoffs across latency, privacy, resilience, and deployment constraints",
        timeoutMs: 8_000,
      },
      (update) => {
        updates.push(update.status);
      },
    );

    expect(updates).toContain("starting");
    expect(updates).toContain("waiting_for_sources");
    expect(updates).toContain("running");
    expect(result.report_text).toContain(
      "Research report for: analyze browser-backed provider tradeoffs across latency, privacy, resilience, and deployment constraints",
    );
    expect(result.report_text).not.toContain(
      "Browser vs Hosted AI for Enterprise",
    );
    expect(result.report_text).not.toContain("Starting...");
    expect(result.report_text).not.toContain("Stop research");

    await adapter.shutdown();
  });

  it("submits research from the /deep-research landing page and completes normally", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          variant: "live",
          scenario: "research-deep-landing",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        scenario: "research-deep-landing",
      }),
      headless: true,
    });

    const updates: string[] = [];
    const result = await adapter.research(
      {
        jobId: "job-research-deep-landing",
        query: "investigate deep research landing submission",
        files: [fixture],
        timeoutMs: 8_000,
      },
      (update) => {
        updates.push(update.status);
      },
    );
    const pageState = await adapter.getManagedPageForTest()!.evaluate(() => ({
      researchLandingVisited: document.body.dataset.researchLandingVisited,
      researchSubmitted: document.body.dataset.researchSubmitted,
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
    }));

    expect(result.report_text).toContain(
      "Research report for: investigate deep research landing submission",
    );
    expect(updates).toContain("starting");
    expect(updates).toContain("waiting_for_sources");
    expect(updates).toContain("running");
    expect(pageState.researchLandingVisited).toBe("true");
    expect(pageState.researchSubmitted).toBe("true");
    expect(["button", "keyboard"]).toContain(pageState.lastSubmissionMethod);

    await adapter.shutdown();
  });

  it("runs a follow-up ask from a finished research conversation after clearing the active Deep research composer chip", async () => {
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

    await adapter.research(
      {
        jobId: "job-research-chip-followup",
        query: "finish a research run before switching back to chat",
        timeoutMs: 8_000,
      },
      () => {},
    );

    await adapter.getManagedPageForTest()!.evaluate(() => {
      const trigger = document.querySelector("#model-switcher-trigger");
      if (!(trigger instanceof HTMLElement)) {
        throw new Error("model trigger missing");
      }
      trigger.hidden = true;

      const chip = document.createElement("button");
      chip.type = "button";
      chip.textContent = "Deep research";
      chip.className = "__composer-pill";
      chip.setAttribute("aria-label", "Deep research, click to remove");
      chip.addEventListener("click", () => {
        chip.remove();
        trigger.hidden = false;
        document.body.dataset.selectedMode = "normal";
        const syncState = (
          window as Window & { syncSelectedState?: () => void }
        ).syncSelectedState;
        if (typeof syncState === "function") {
          syncState();
        }
      });

      const composer = document.querySelector("[data-testid='composer-root']");
      composer?.appendChild(chip);
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-chip-followup",
        prompt: "reply from a finished research thread",
        modelClass: "thinking",
        effort: "standard",
        timeoutMs: 7_000,
      },
      () => {},
    );

    expect(result.output_text).toContain(
      "Fake response for: reply from a finished research thread",
    );

    await adapter.shutdown();
  });

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
