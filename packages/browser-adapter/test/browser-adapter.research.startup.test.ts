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
  openTrackedManagedAdapter,
} from "./helpers/browserFlow";
import {
  beginDeepResearchRun,
  readDeepResearchFrameSnapshot,
} from "../src/pageRuntime";
import { installBrowserAdapterHarness } from "./helpers/browserAdapterHarness";
import {
  fakeArtifactPage,
  fakeResearchHost,
  fakeResearchSnapshot,
} from "./helpers/researchFinalization";

const harness = installBrowserAdapterHarness();

describe("browser adapter research: startup and configuration", () => {
  it("runs research jobs with progress updates and persisted artifacts", async () => {
    const server = await harness.startServer();
    const { adapter, root } = await openManagedHarnessAdapter(
      harness,
      server.url,
      {
        variant: "live",
        scenario: "research-success",
      },
    );
    const fixtureDir = path.join(root, "fixtures");
    const firstFile = path.join(fixtureDir, "brief.txt");
    const secondFile = path.join(fixtureDir, "notes.csv");
    await mkdir(fixtureDir, { recursive: true });
    await writeFile(firstFile, "brief body", "utf8");
    await writeFile(secondFile, "col\nvalue", "utf8");

    const progress = captureResearchStatuses();
    const result = await adapter.research(
      {
        jobId: "job-research-success",
        query: "investigate launch risks",
        files: [firstFile, secondFile],
        timeoutMs: 5_000,
      },
      progress.onUpdate,
    );

    expectResearchHandoffStatuses(progress.statuses);
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
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      variant: "live",
      scenario: "research-starting-shell",
    });

    const progress = captureResearchStatuses();
    const result = await adapter.research(
      {
        jobId: "job-research-starting-shell",
        query:
          "analyze browser-backed provider tradeoffs across latency, privacy, resilience, and deployment constraints",
        timeoutMs: 8_000,
      },
      progress.onUpdate,
    );

    expectResearchHandoffStatuses(progress.statuses);
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
    const { adapter, root } = await openManagedHarnessAdapter(
      harness,
      server.url,
      {
        variant: "live",
        scenario: "research-deep-landing",
      },
    );
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");

    const progress = captureResearchStatuses();
    const result = await adapter.research(
      {
        jobId: "job-research-deep-landing",
        query: "investigate deep research landing submission",
        files: [fixture],
        timeoutMs: 8_000,
      },
      progress.onUpdate,
    );
    const pageState = await adapter.getManagedPageForTest()!.evaluate(() => ({
      researchLandingVisited: document.body.dataset.researchLandingVisited,
      researchSubmitted: document.body.dataset.researchSubmitted,
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
    }));

    expect(result.report_text).toContain(
      "Research report for: investigate deep research landing submission",
    );
    expectResearchHandoffStatuses(progress.statuses);
    expect(pageState.researchLandingVisited).toBe("true");
    expect(pageState.researchSubmitted).toBe("true");
    expect(["button", "keyboard"]).toContain(pageState.lastSubmissionMethod);

    await adapter.shutdown();
  });

  it("enters live Deep Research by URL without clicking the sidebar nav", async () => {
    const server = await harness.startServer();
    const target = new URL(
      withHarnessUrl(server.url, {
        variant: "live",
        scenario: "research-success",
      }),
    );
    target.pathname = "/c/existing-research-thread";
    const { adapter } = await openTrackedManagedAdapter(
      harness,
      target.toString(),
    );
    await adapter.getManagedPageForTest()!.evaluate(() => {
      const storageKey = "tablinqDeepResearchNavClicks";
      localStorage.setItem(storageKey, "0");
      document
        .querySelector("#deep-research-nav")
        ?.addEventListener("click", () => {
          localStorage.setItem(
            storageKey,
            String(Number(localStorage.getItem(storageKey) ?? "0") + 1),
          );
        });
    });

    const result = await adapter.research(
      {
        jobId: "job-research-no-sidebar-click",
        query: "check deep research without sidebar activation",
        timeoutMs: 8_000,
      },
      () => {},
    );
    const navClicks = await adapter
      .getManagedPageForTest()!
      .evaluate(() => localStorage.getItem("tablinqDeepResearchNavClicks"));

    expect(result.report_text).toContain(
      "Research report for: check deep research without sidebar activation",
    );
    expect(navClicks).toBe("0");

    await adapter.shutdown();
  });

  it("clicks the updated main-frame Deep Research Start control", async () => {
    const server = await harness.startServer();
    const { adapter, targetUrl } = await openManagedHarnessAdapter(
      harness,
      server.url,
      {
        variant: "live",
        scenario: "research-success",
      },
    );
    const page = adapter.getManagedPageForTest()!;
    const deepUrl = new URL(targetUrl);
    deepUrl.pathname = "/deep-research";
    await page.goto(deepUrl.toString(), { waitUntil: "domcontentloaded" });
    await page.evaluate(() => {
      document.body.innerHTML = `
        <main data-deep-research-root="true">
          <h1>Deep research</h1>
          <p>Review the research plan.</p>
          <button type="button" id="start-research">Start 23</button>
        </main>
      `;
      document
        .querySelector("#start-research")
        ?.addEventListener("click", () => {
          document.body.innerHTML = `
          <main data-deep-research-root="true">
            <h1>Deep research</h1>
            <p data-deep-research-status="true">gathering sources</p>
            <pre data-deep-research-text="true">Research report started</pre>
          </main>
        `;
        });
    });

    const before = await readDeepResearchFrameSnapshot(page);
    expect(before).toMatchObject({
      phase: "planning",
      startVisible: true,
      startEnabled: true,
    });
    const started = await beginDeepResearchRun(page, 2_000);

    expect(started).toMatchObject({
      status: "waiting_for_sources",
      statusMessage: "gathering sources",
      reportText: "Research report started",
    });

    await adapter.shutdown();
  });

  it("waits through a Deep Research planner countdown shell before start", async () => {
    const server = await harness.startServer();
    const { adapter, targetUrl } = await openManagedHarnessAdapter(
      harness,
      server.url,
      {
        variant: "live",
        scenario: "research-success",
      },
    );
    const page = adapter.getManagedPageForTest()!;
    const deepUrl = new URL(targetUrl);
    deepUrl.pathname = "/deep-research";
    await page.goto(deepUrl.toString(), { waitUntil: "domcontentloaded" });
    await page.evaluate(() => {
      document.body.innerHTML = `
        <main data-deep-research-root="true">
          <h1>Research outline</h1>
          <p>Review the proposed research plan.</p>
          <button type="button">Edit</button>
          <button type="button">Cancel</button>
          <p>Plan starts in 57 seconds.</p>
          <span>57</span>
        </main>
      `;
      window.setTimeout(() => {
        document.body.innerHTML = `
          <main data-deep-research-root="true">
            <h1>Deep research</h1>
            <p data-deep-research-status="true">gathering sources</p>
            <pre data-deep-research-text="true">Research report started</pre>
          </main>
        `;
      }, 200);
    });

    const before = await readDeepResearchFrameSnapshot(page);
    expect(before).toMatchObject({
      phase: "planning",
      startVisible: false,
    });
    const started = await beginDeepResearchRun(page, 2_000);

    expect(started).toMatchObject({
      status: "waiting_for_sources",
      statusMessage: "gathering sources",
      reportText: "Research report started",
    });

    await adapter.shutdown();
  });

  it("reads completed Deep Research reports from the nested sandbox document", async () => {
    const server = await harness.startServer();
    const { adapter, targetUrl } = await openManagedHarnessAdapter(
      harness,
      server.url,
      {
        variant: "live",
        scenario: "research-success",
      },
    );
    const page = adapter.getManagedPageForTest()!;
    const deepUrl = new URL(targetUrl);
    deepUrl.pathname = "/deep-research";
    await page.goto(deepUrl.toString(), { waitUntil: "domcontentloaded" });
    await page.evaluate(() => {
      document.body.innerHTML = `
        <iframe id="root" src="about:blank"></iframe>
      `;
      const nested = document.querySelector("#root");
      const doc =
        nested instanceof HTMLIFrameElement ? nested.contentDocument : null;
      if (!doc) {
        throw new Error("nested sandbox document was not available");
      }
      doc.open();
      doc.write(`
        <main>
          <div class="text-token-text-secondary">Research completed in 12m · 4 citations · 8 searches</div>
          <section class="_reportPage_test">
            <h1>Nested report title</h1>
            <h2>Executive summary</h2>
            <p>Nested sandbox report body that should be returned.</p>
          </section>
        </main>
      `);
      doc.close();
    });

    const snapshot = await readDeepResearchFrameSnapshot(page);

    expect(snapshot).toMatchObject({
      phase: "done",
      statusMessage: "Research completed in 12m · 4 citations · 8 searches",
      reportText:
        "Nested report title\nExecutive summary\n\nNested sandbox report body that should be returned.",
    });

    await adapter.shutdown();
  });

  it("does not duplicate research submit after a keyboard submit edge is detected", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "research-success",
    });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });
    const page = adapter.getManagedPageForTest()!;
    await page.addInitScript(() => {
      const installKeyboardOnlySubmit = () => {
        document.querySelector("[data-testid='send-button']")?.remove();
        if (!document.body) {
          return;
        }
        document.body.dataset.keyboardSubmitReady = "true";
        const composer = document.querySelector("#prompt-textarea");
        if (!(composer instanceof HTMLElement)) {
          return;
        }
        if (composer.dataset.tablinqKeyboardCounterInstalled === "true") {
          return;
        }
        composer.dataset.tablinqKeyboardCounterInstalled = "true";
        composer.addEventListener("keydown", (event) => {
          if (
            event instanceof KeyboardEvent &&
            event.key === "Enter" &&
            (event.ctrlKey || event.metaKey)
          ) {
            document.body.dataset.keyboardSubmitEvents = String(
              Number(document.body.dataset.keyboardSubmitEvents ?? "0") + 1,
            );
          }
        });
      };
      new MutationObserver(installKeyboardOnlySubmit).observe(document, {
        childList: true,
        subtree: true,
      });
      document.addEventListener("DOMContentLoaded", installKeyboardOnlySubmit);
      installKeyboardOnlySubmit();
    });
    await page.evaluate(() => {
      document.querySelector("[data-testid='send-button']")?.remove();
      document.body.dataset.keyboardSubmitReady = "true";
      const composer = document.querySelector("#prompt-textarea");
      if (!(composer instanceof HTMLElement)) {
        return;
      }
      if (composer.dataset.tablinqKeyboardCounterInstalled === "true") {
        return;
      }
      composer.dataset.tablinqKeyboardCounterInstalled = "true";
      document
        .querySelector("#prompt-textarea")
        ?.addEventListener("keydown", (event) => {
          if (
            event instanceof KeyboardEvent &&
            event.key === "Enter" &&
            (event.ctrlKey || event.metaKey)
          ) {
            document.body.dataset.keyboardSubmitEvents = String(
              Number(document.body.dataset.keyboardSubmitEvents ?? "0") + 1,
            );
          }
        });
    });

    const result = await adapter.research(
      {
        jobId: "job-research-keyboard-submit-single-edge",
        query: "submit research through keyboard once",
        timeoutMs: 8_000,
      },
      () => {},
    );
    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      keyboardSubmitEvents: document.body.dataset.keyboardSubmitEvents ?? "0",
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
      researchSubmitted: document.body.dataset.researchSubmitted,
    }));

    expect(result.report_text).toContain(
      "Research report for: submit research through keyboard once",
    );
    expect(state).toEqual({
      keyboardSubmitEvents: "1",
      lastSubmissionMethod: "keyboard",
      researchSubmitted: "true",
    });

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
});
