import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import {
  BrowserAdapter,
  chooseUniqueArtifactPath,
  conversationIdFromUrl,
  deepResearchStartDelayMs,
  detectBrowserExecutable,
  interactionJitterMs,
  MAX_UPLOAD_FILE_BYTES,
  providerPressureCooldownMs,
  resolveConversationUrl,
  resolveBrowserLaunchConfig,
  uploadAckTimeoutMsForFiles,
  workspaceKeyFromUrl,
} from "../src/browserAdapter";
import type { StreamChunkSnapshot } from "../src/browserAdapter";
import { finalizeAsk } from "../src/execution/finalization";
import { applyAskSnapshot } from "../src/execution/streaming";
import {
  createIsolatedTempDir,
  listAttachedPages,
  withAttachedBrowser,
  withHarnessUrl,
} from "./browser-adapter.integration.helpers";
import { stopFakeChatGptServer } from "./helpers";
import {
  createAskRuntimeDom,
  liveActualAnswerText,
  liveStyleAnswerMessage,
  liveStyleReasoningTurn,
  liveThoughtArtifactText,
} from "./helpers/askRuntimeDom";
import {
  captureDeltaChunks,
  expectCompletedAskResult,
  openManagedHarnessAdapter,
  openTrackedManagedAdapter,
} from "./helpers/browserFlow";
import { installBrowserAdapterHarness } from "./helpers/browserAdapterHarness";

const harness = installBrowserAdapterHarness();

async function withShortAskStreamWatchdog<T>(
  run: () => Promise<T>,
): Promise<T> {
  const previousInterval = process.env.TABLINQ_ASK_STREAM_SNAPSHOT_INTERVAL_MS;
  const previousMax = process.env.TABLINQ_ASK_STREAM_MAX_MS;
  const previousSticky = process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS;
  process.env.TABLINQ_ASK_STREAM_SNAPSHOT_INTERVAL_MS = "150";
  process.env.TABLINQ_ASK_STREAM_MAX_MS = "5000";
  process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS = "5000";
  try {
    return await run();
  } finally {
    if (previousInterval === undefined) {
      delete process.env.TABLINQ_ASK_STREAM_SNAPSHOT_INTERVAL_MS;
    } else {
      process.env.TABLINQ_ASK_STREAM_SNAPSHOT_INTERVAL_MS = previousInterval;
    }
    if (previousMax === undefined) {
      delete process.env.TABLINQ_ASK_STREAM_MAX_MS;
    } else {
      process.env.TABLINQ_ASK_STREAM_MAX_MS = previousMax;
    }
    if (previousSticky === undefined) {
      delete process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS;
    } else {
      process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS = previousSticky;
    }
  }
}

describe("browser adapter ask: submit readiness and surface locking", () => {
  it("keeps the Mermaid preview fallback when no source code is available", async () => {
    const { adapter, dom, main } = await createAskRuntimeDom();
    const baseline = adapter.readAskBaseline();
    const svg = encodeURIComponent(`
      <svg xmlns="http://www.w3.org/2000/svg">
        <text>Start</text>
        <text>Process</text>
        <text>Done</text>
      </svg>
    `);
    main.insertAdjacentHTML(
      "beforeend",
      `<section data-testid="conversation-turn-mermaid">
        <h4 class="sr-only">ChatGPT said:</h4>
        <div data-message-author-role="assistant" data-message-id="mermaid-preview-msg">
          <div class="markdown">
            <pre>
              <button aria-label="Code" type="button">Code</button>
              <button aria-label="Preview" type="button">Preview</button>
              <img alt="Mermaid preview" src="data:image/svg+xml,${svg}">
            </pre>
          </div>
        </div>
      </section>`,
    );
    dom.window.document.body.dataset.streamState = "done";

    const snapshot = adapter.readAskSnapshot({ baseline });

    expect(snapshot.text).toBe(
      "Mermaid diagram preview: Start; Process; Done.",
    );
    dom.window.close();
  });

  it("falls back to keyboard submit when a composer plus button sits next to send", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "submit-upload-trap",
    });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const result = await adapter.ask(
      {
        jobId: "job-ask-submit-upload-trap",
        prompt: "repeat attached file",
        files: [fixture],
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      uploadPopoverOpen: document.body.dataset.uploadPopoverOpen,
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
      uploadChips: document.querySelectorAll("[data-testid='upload-chip']")
        .length,
      plusPresent: Boolean(document.querySelector("#composer-plus-btn")),
    }));

    expect(result.output_text).toBe("Fake response for: repeat attached file");
    expect(state).toEqual({
      uploadPopoverOpen: "false",
      lastSubmissionMethod: "keyboard",
      uploadChips: 1,
      plusPresent: true,
    });

    await adapter.shutdown();
  });

  it("does not duplicate submit after a keyboard submit edge is detected", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, { variant: "live" }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { variant: "live" }),
      headless: true,
    });

    await adapter.getManagedPageForTest()!.evaluate(() => {
      const send = document.querySelector("[data-testid='send-button']");
      if (send instanceof HTMLElement) {
        send.remove();
      }
      document.body.dataset.keyboardSubmitReady = "true";
      let keyboardSubmits = 0;
      document
        .querySelector("#prompt-textarea")
        ?.addEventListener("keydown", (event) => {
          if (
            event instanceof KeyboardEvent &&
            event.key === "Enter" &&
            (event.ctrlKey || event.metaKey)
          ) {
            keyboardSubmits += 1;
            document.body.dataset.keyboardSubmitEvents =
              String(keyboardSubmits);
          }
        });
    });

    const result = await adapter.ask(
      {
        jobId: "job-ask-keyboard-submit-single-edge",
        prompt: "submit through keyboard once",
        timeoutMs: 7_000,
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      keyboardSubmitEvents: document.body.dataset.keyboardSubmitEvents ?? "0",
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
    }));

    expect(result.output_text).toBe(
      "Fake response for: submit through keyboard once",
    );
    expect(state).toEqual({
      keyboardSubmitEvents: "1",
      lastSubmissionMethod: "keyboard",
    });

    await adapter.shutdown();
  });

  it("waits for the quiet surface before a high-risk submit click", async () => {
    const previousPacing = process.env.TABLINQ_HIGH_RISK_PACING;
    process.env.TABLINQ_HIGH_RISK_PACING = "true";

    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, { variant: "live" });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );

    try {
      await adapter.open({ targetUrl, headless: true });
      await adapter.getManagedPageForTest()!.evaluate(() => {
        const rootElement = document.querySelector(
          "[data-testid='composer-root']",
        );
        const send = document.querySelector("[data-testid='send-button']");
        if (
          !(rootElement instanceof HTMLElement) ||
          !(send instanceof HTMLElement)
        ) {
          throw new Error("composer surface missing");
        }

        let tick = 0;
        const interval = window.setInterval(() => {
          rootElement.dataset.quietSubmitProbe = String(tick);
          tick += 1;
          if (tick >= 12) {
            document.body.dataset.quietMutationEnd = String(performance.now());
            window.clearInterval(interval);
          }
        }, 40);
        send.addEventListener("click", () => {
          document.body.dataset.submitClickAt = String(performance.now());
        });
      });

      await adapter.ask(
        {
          jobId: "job-ask-quiet-surface-submit",
          prompt: "wait before submitting",
          timeoutMs: 8_000,
        },
        () => {},
      );

      const timing = await adapter.getManagedPageForTest()!.evaluate(() => ({
        mutationEnd: Number(document.body.dataset.quietMutationEnd ?? "0"),
        submitClickAt: Number(document.body.dataset.submitClickAt ?? "0"),
      }));

      expect(timing.mutationEnd).toBeGreaterThan(0);
      expect(timing.submitClickAt - timing.mutationEnd).toBeGreaterThanOrEqual(
        150,
      );
    } finally {
      if (previousPacing === undefined) {
        delete process.env.TABLINQ_HIGH_RISK_PACING;
      } else {
        process.env.TABLINQ_HIGH_RISK_PACING = previousPacing;
      }
      await adapter.shutdown();
    }
  });

  it("rejects overlapping interactive actions on the same browser surface", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, { scenario: "timeout" });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    await withShortAskStreamWatchdog(async () => {
      const first = adapter.ask(
        {
          jobId: "job-ask-single-flight-first",
          prompt: "hold the browser surface",
          timeoutMs: 600,
        },
        () => {},
      );
      const second = adapter.ask(
        {
          jobId: "job-ask-single-flight-second",
          prompt: "overlap on the same surface",
          timeoutMs: 600,
        },
        () => {},
      );

      await expect(second).rejects.toThrow(/browser surface busy/i);
      await expect(first).rejects.toThrow(/timed out/i);
    });

    await adapter.shutdown();
  });
});
