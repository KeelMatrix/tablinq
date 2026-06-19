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

describe("browser adapter recovery: ask streaming recovery", () => {
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
        connectionMode: "managed_only",
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
        connectionMode: "managed_only",
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
        timeoutMs: 15_000,
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
        connectionMode: "managed_only",
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
        submittedToProvider: false,
        submittedConversationId: null,
        askBaseline: {
          assistantMessageCount: 0,
          conversationId: first.conversation_id,
          messageId: null,
          text: "",
        },
        pendingTextReconciliation: false,
        lastProgressAt: Date.now(),
        lastTextChangeAt: Date.now(),
        lastObservedText: "",
        stickyStreamReloaded: false,
        stickyStreamFallbackAt: null,
        recoveryAttempts: 0,
        lastStatusAt: 0,
        lastSnapshotSignature: null,
        lastProviderActivitySignature: null,
        lastProviderActivityAt: null,
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


  it("does not open a fresh conversation while reconciling a submitted ask without a known conversation", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "managed_only",
      }),
    );
    await adapter.open({ targetUrl: server.url, headless: true });
    const adapterWithInternals = adapter as any;
    const openExecutionPage = vi.spyOn(
      adapterWithInternals.execution.host,
      "openExecutionPage",
    );

    await expect(
      adapterWithInternals.reconcileAskAfterInterruption(
        adapter.getManagedPageForTest()!,
        {
          jobId: "job-submitted-unknown-conversation",
          prompt: "already submitted",
          timeoutMs: 2_000,
        },
        {
          emittedText: "",
          nextSeq: 0,
          doneStreak: 0,
          knownConversationId: null,
          knownMessageId: null,
          submittedToProvider: true,
          submittedConversationId: null,
          askBaseline: {
            assistantMessageCount: 0,
            conversationId: null,
            messageId: null,
            text: "",
          },
          pendingMessageId: null,
          pendingMessageText: "",
          pendingTextReconciliation: false,
          lastProgressAt: Date.now() - 10_000,
          lastTextChangeAt: Date.now() - 10_000,
          lastObservedText: "",
          stickyStreamReloaded: false,
          stickyStreamFallbackAt: null,
          recoveryAttempts: 0,
          lastStatusAt: 0,
          lastSnapshotSignature: null,
          lastProviderActivitySignature: null,
          lastProviderActivityAt: null,
          lastIdentityUpdateSignature: null,
          transientInterruptionThoughtArtifactSignature: null,
        },
        () => {},
        () => {},
        "recover submitted ask",
        Date.now() + 2_000,
      ),
    ).rejects.toThrow(
      /submitted ask cannot be reconciled because no provider conversation id is known/i,
    );

    expect(openExecutionPage).not.toHaveBeenCalled();
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
        connectionMode: "managed_only",
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
    expect(states).toContain("running");

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
        connectionMode: "managed_only",
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
    expect(states).toContain("running");

    await adapter.shutdown();
  });
});
