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
  createBrowserIntegrationHarness,
  createIsolatedTempDir,
  listAttachedPages,
  withAttachedBrowser,
  withHarnessUrl,
} from "./browser-adapter.integration.helpers";
import { openManagedHarnessAdapter } from "./helpers/browserFlow";

let harness: ReturnType<typeof createBrowserIntegrationHarness>;

beforeEach(() => {
  harness = createBrowserIntegrationHarness();
});

afterEach(async () => {
  await harness.cleanup();
});

async function maybeCloseManagedBrowserOnRunningIdentity(
  adapter: BrowserAdapter,
  update: {
    state: string;
    conversation_id?: string | null;
    message_id?: string | null;
  },
  alreadyClosed: boolean,
): Promise<boolean> {
  if (
    alreadyClosed ||
    update.state !== "running" ||
    (!update.message_id && !update.conversation_id)
  ) {
    return alreadyClosed;
  }
  await adapter.getManagedPageForTest()!.context().browser()!.close();
  return true;
}

describe("browser adapter", () => {
  it("backs off adaptive polling during longer recovery waits", () => {
    const immediate = adaptivePollIntervalMs("recovery", 100, 1_000);
    const settled = adaptivePollIntervalMs("recovery", 2_000, 1_000);
    const prolonged = adaptivePollIntervalMs("recovery", 6_000, 1_000);

    expect(immediate).toBeLessThan(settled);
    expect(settled).toBeLessThan(prolonged);
  });

  it("treats inserted markdown list lead-ins as compatible during ask recovery", () => {
    const emitted = [
      "Here are notable bakeries in Đà Lạt with strong local popularity and reviews:",
      "",
      "### 🥐 Popular bakeries",
      "",
      "- Large, well-known local bakery chain with a wide",
    ].join("\n");
    const final = [
      "Here are notable bakeries in Đà Lạt with strong local popularity and reviews:",
      "",
      "### 🥐 Popular bakeries",
      "",
      "- Lien Hoa Bakery",
      "  Large, well-known local bakery chain with a wide selection of breads, cakes, and Vietnamese pastries.",
    ].join("\n");

    expect(markdownCompatibleRecoveredText(emitted, final)).toBe(true);
  });

  it("does not treat unrelated inserted markdown content as compatible recovery text", () => {
    const emitted = [
      "Here are notable bakeries in Đà Lạt with strong local popularity and reviews:",
      "",
      "### 🥐 Popular bakeries",
      "",
      "- Large, well-known local bakery chain with a wide",
    ].join("\n");
    const final = [
      "Here are notable bakeries in Đà Lạt with strong local popularity and reviews:",
      "",
      "### 🥐 Popular bakeries",
      "",
      "- Unexpected Bakery",
      "  Known only for coffee, not a wide bakery selection.",
    ].join("\n");

    expect(markdownCompatibleRecoveredText(emitted, final)).toBe(false);
  });

  it("continues ask streaming after a page reload", async () => {
    const server = await harness.startServer();
    const { adapter } = await openManagedHarnessAdapter(harness, server.url, {
      scenario: "reload",
    });

    const states: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-reload",
        prompt: "survive reloads",
        timeoutMs: 5000,
      },
      () => {},
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.output_text).toBe("Fake response for: survive reloads");
    expect((await adapter.health()).reload_count).toBeGreaterThan(0);

    await adapter.shutdown();
  });

  it("recovers ask streaming after a browser disconnect", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
      }),
    );
    await adapter.open({ targetUrl: server.url, headless: true });

    const states: string[] = [];
    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-browser-disconnect",
        prompt: "recover after browser disconnect",
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
      "Fake response for: recover after browser disconnect",
    );
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
  });

  it("recovers a follow-up ask in the same conversation without replaying the previous reply", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, {
      scenario: "followup-stale",
    });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const first = await adapter.ask(
      {
        jobId: "job-ask-followup-recovery-first",
        prompt: "hello gpt",
      },
      () => {},
    );

    const states: string[] = [];
    const secondChunks: string[] = [];
    let closeTriggered = false;
    const request = adapter.ask(
      {
        jobId: "job-ask-followup-recovery-second",
        prompt: "recover the follow-up ask",
        conversationId: first.conversation_id,
        timeoutMs: 7000,
      },
      async (chunk) => {
        if (chunk.kind === "delta") {
          secondChunks.push(chunk.text);
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

    const second = await request;
    expect(second.conversation_id).toBe(first.conversation_id);
    expect(second.output_text).toBe(
      "Fake response for: recover the follow-up ask",
    );
    expect(secondChunks.join("")).toBe(second.output_text);
    expect(secondChunks.join("")).not.toContain(first.output_text);
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
  });

  it("recovers a follow-up ask from the ask baseline conversation when the cached target URL is stale", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, {
      scenario: "followup-stale",
    });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const first = await adapter.ask(
      {
        jobId: "job-ask-followup-baseline-first",
        prompt: "hello gpt",
      },
      () => {},
    );
    await adapter.getManagedPageForTest()!.evaluate(() => {
      document.body.dataset.streamState = "done";
      const latest = Array.from(
        document.querySelectorAll("[data-message-author-role='assistant']"),
      ).at(-1);
      if (latest instanceof HTMLElement) {
        latest.setAttribute("data-stream-state", "done");
      }
    });

    const adapterWithInternals = adapter as any;
    adapterWithInternals.currentTargetUrl = server.url;

    const chunks: string[] = [];
    const result = await adapterWithInternals.reconcileAskAfterInterruption(
      adapter.getManagedPageForTest()!,
      {
        jobId: "job-ask-followup-baseline-recover",
        prompt: "hello gpt",
        timeoutMs: 7_000,
      },
      {
        emittedText: "",
        nextSeq: 0,
        doneStreak: 0,
        knownConversationId: null,
        knownMessageId: null,
        askBaseline: {
          assistantMessageCount: 0,
          conversationId: first.conversation_id,
          messageId: null,
          text: "",
        },
        pendingTextReconciliation: false,
        lastProgressAt: Date.now(),
        recoveryAttempts: 0,
        lastStatusAt: 0,
        lastSnapshotSignature: null,
        lastIdentityUpdateSignature: null,
      },
      (chunk: any) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
      () => {},
      "recovered the interrupted ask",
      Date.now() + 7_000,
    );

    expect(result.result?.conversation_id).toBe(first.conversation_id);
    expect(result.result?.output_text).toBe(first.output_text);
    expect(chunks.join("")).toBe(first.output_text);

    await adapter.shutdown();
  });

  it("recovers from a temporary streaming stall without replaying the prompt", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, {
          scenario: "stall-recover",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { scenario: "stall-recover" }),
      headless: true,
    });

    const chunks: string[] = [];
    const states: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-stall-recover",
        prompt: "resume after a temporary stall",
        timeoutMs: 6000,
      },
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.output_text).toBe(
      "Fake response for: resume after a temporary stall",
    );
    expect(chunks.join("")).toBe(result.output_text);
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
  });

  it("recovers ask when the live page is transiently ambiguous during stall recovery", async () => {
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
          scenario: "stall-recover-transient",
        }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, {
        variant: "live",
        scenario: "stall-recover-transient",
      }),
      headless: true,
    });

    const chunks: string[] = [];
    const states: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-stall-recover-transient",
        prompt: "recover after a transient live page",
        files: [fixture],
        timeoutMs: 7000,
      },
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
      (update) => {
        states.push(update.state);
      },
    );

    expect(result.output_text).toBe(
      "Fake response for: recover after a transient live page",
    );
    expect(result.artifacts).toHaveLength(1);
    expect(chunks.join("")).toBe(result.output_text);
    expect(result.current_url).toContain(`/c/${result.conversation_id}`);
    expect(states).toContain("interrupted");
    expect(states).toContain("recovered");

    await adapter.shutdown();
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
