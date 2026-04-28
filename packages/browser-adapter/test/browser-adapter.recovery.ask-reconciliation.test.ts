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

describe("browser adapter recovery: ask reconciliation recovery", () => {
  it("reloads once and returns when a completed ask still looks like it is streaming", async () => {
    const previousStall = process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS;
    const previousGrace = process.env.TABLINQ_ASK_STICKY_STREAM_FINAL_GRACE_MS;
    process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS = "250";
    process.env.TABLINQ_ASK_STICKY_STREAM_FINAL_GRACE_MS = "150";

    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      scenario: "sticky-stop-after-complete",
    });
    const { adapter } = await openManagedHarnessAdapter(harness, targetUrl);

    try {
      const chunks: string[] = [];
      const result = await adapter.ask(
        {
          jobId: "job-ask-sticky-stop-reload-final",
          prompt: "finish but leave stop visible",
          timeoutMs: 5_000,
        },
        (chunk) => {
          if (chunk.kind === "delta") {
            chunks.push(chunk.text);
          }
        },
      );

      expect(result.output_text).toBe(
        "Fake response for: finish but leave stop visible",
      );
      expect(chunks.join("")).toBe(result.output_text);
      expect((await adapter.health()).reload_count).toBeGreaterThan(0);
    } finally {
      if (previousStall === undefined) {
        delete process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS;
      } else {
        process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS = previousStall;
      }
      if (previousGrace === undefined) {
        delete process.env.TABLINQ_ASK_STICKY_STREAM_FINAL_GRACE_MS;
      } else {
        process.env.TABLINQ_ASK_STICKY_STREAM_FINAL_GRACE_MS = previousGrace;
      }
      await adapter.shutdown();
    }
  });

  it("returns the streamed ask text after sticky streaming remains running after reload", async () => {
    const previousStall = process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS;
    const previousGrace = process.env.TABLINQ_ASK_STICKY_STREAM_FINAL_GRACE_MS;
    process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS = "250";
    process.env.TABLINQ_ASK_STICKY_STREAM_FINAL_GRACE_MS = "150";

    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      scenario: "sticky-stop-after-reload",
    });
    const { adapter } = await openManagedHarnessAdapter(harness, targetUrl);

    try {
      const result = await adapter.ask(
        {
          jobId: "job-ask-sticky-stop-reload-partial",
          prompt: "return after sticky stop survives reload",
          timeoutMs: 5_000,
        },
        () => {},
      );

      expect(result.output_text).toBe(
        "Fake response for: return after sticky stop survives reload",
      );
      expect((await adapter.health()).reload_count).toBeGreaterThan(0);
    } finally {
      if (previousStall === undefined) {
        delete process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS;
      } else {
        process.env.TABLINQ_ASK_STICKY_STREAM_RELOAD_STALL_MS = previousStall;
      }
      if (previousGrace === undefined) {
        delete process.env.TABLINQ_ASK_STICKY_STREAM_FINAL_GRACE_MS;
      } else {
        process.env.TABLINQ_ASK_STICKY_STREAM_FINAL_GRACE_MS = previousGrace;
      }
      await adapter.shutdown();
    }
  });

  it("recovers a final result that completed upstream while disconnected", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "recover-complete",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { scenario: "recover-complete" }),
      headless: true,
    });

    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-recover-complete",
        prompt: "complete while disconnected",
        timeoutMs: 7000,
      },
      () => {},
      async (update) => {
        closeTriggered = await maybeCloseManagedBrowserOnRunningIdentity(
          adapter,
          update,
          closeTriggered,
        );
      },
    );

    const result = await request;
    expect(result.output_text).toBe(
      "Fake response for: complete while disconnected",
    );
    expect(result.current_url).toContain(`/c/${result.conversation_id}`);
    expect(conversationIdFromUrl(result.current_url)).toBe(
      result.conversation_id,
    );

    await adapter.shutdown();
  });

  it("recovers a final ask after transient placeholder drift stabilizes", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "recover-done-transient",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        scenario: "recover-done-transient",
      }),
      headless: true,
    });

    const states: string[] = [];
    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-recover-done-transient",
        prompt: "Reply with exactly: ASK_OK",
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
      "Fake response for: Reply with exactly: ASK_OK",
    );
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
  });

  it("recovers when the assistant message id rotates to a later response in the same conversation", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "recover-message-successor",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        scenario: "recover-message-successor",
      }),
      headless: true,
    });

    const states: string[] = [];
    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-recover-message-successor",
        prompt: "recover after assistant message rotation",
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
      "Fake response for: recover after assistant message rotation",
    );
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
  });

  it("recovers an in-flight ask mismatch that later converges on the same message", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "recover-running-transient",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        scenario: "recover-running-transient",
      }),
      headless: true,
    });

    const states: string[] = [];
    const chunks: string[] = [];
    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-recover-running-transient",
        prompt: "recover after placeholder drift",
        timeoutMs: 7000,
      },
      async (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
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
      "Fake response for: recover after placeholder drift",
    );
    expect(chunks.join("")).toBe(result.output_text);
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
  });

  it("recovers when the same assistant message rewrites provisional thinking text into the final answer", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "recover-same-message-rewrite",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        scenario: "recover-same-message-rewrite",
      }),
      headless: true,
    });

    const states: string[] = [];
    const chunks: string[] = [];
    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-recover-same-message-rewrite",
        prompt: "recover after thinking rewrite",
        timeoutMs: 7_000,
      },
      async (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
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
      "Fake response for: recover after thinking rewrite",
    );
    expect(chunks.join("")).not.toContain("Thought for 7s");
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
  });

  it("recovers a finished ask when the saved emitted text is only transient thinking status", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
      }),
    );
    await adapter.open({
      targetUrl: server.url,
      headless: true,
    });

    const prompt = "recover after transient thinking status";
    const initial = await adapter.ask(
      {
        jobId: "job-ask-transient-thinking-status-initial",
        prompt,
        timeoutMs: 7_000,
      },
      () => undefined,
      () => undefined,
    );

    const recovered = await adapter.recoverAsk(
      {
        jobId: "job-ask-transient-thinking-status-recover",
        prompt,
        conversationId: initial.conversation_id,
        timeoutMs: 7_000,
      },
      {
        emittedText: "Thought for 7s",
        nextSeq: 0,
      },
      () => undefined,
      () => undefined,
    );

    expect(recovered.output_text).toBe(initial.output_text);

    await adapter.shutdown();
  });

  it("recovers a same-message ask when the final live DOM becomes rich markdown", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "recover-rich-markdown",
    });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({
      targetUrl,
      headless: true,
    });

    const states: string[] = [];
    let closeTriggered = false;
    const result = await adapter.ask(
      {
        jobId: "job-ask-recover-rich-markdown",
        prompt: "recover copyable block after interruption",
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

    expect(result.output_text).toBe(
      [
        "```",
        "Summarize release notes into a concise update with risks and actions.",
        "```",
        "",
        "**Outline**",
        "",
        "1. Goal",
        "2. Inputs",
        "3. Constraints",
        "4. Output",
      ].join("\n"),
    );
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
  });

  it("recovers an interrupted ask from same-conversation identity without emitted text", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "recover-mismatch",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { scenario: "recover-mismatch" }),
      headless: true,
    });

    let closeTriggered = false;
    const states: string[] = [];
    const request = adapter.ask(
      {
        jobId: "job-ask-recover-mismatch",
        prompt: "fail recovery on mismatched output",
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
    expect(result.output_text).toBe("Unexpected response after interruption");
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
  });

  it("fails when interrupted ask recovery lands on a different conversation", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "recover-wrong-conversation",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        scenario: "recover-wrong-conversation",
      }),
      headless: true,
    });

    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-recover-wrong-conversation",
        prompt: "fail recovery on wrong conversation",
        timeoutMs: 7000,
      },
      () => {},
      async (update) => {
        closeTriggered = await maybeCloseManagedBrowserOnRunningIdentity(
          adapter,
          update,
          closeTriggered,
        );
      },
    );

    await expect(request).rejects.toThrow(/could not be reconciled/i);

    await adapter.shutdown();
  });
});
