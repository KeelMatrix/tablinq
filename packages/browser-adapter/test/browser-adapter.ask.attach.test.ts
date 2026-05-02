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

describe("browser adapter ask: attach mode", () => {
  it("streams ask responses to completion in attach mode", async () => {
    const server = await harness.startServer();
    const adapter = await harness.createAdapterForMode("attach", server.url);
    await adapter.open({ targetUrl: server.url, headless: true });

    const chunks: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-success-attach",
        prompt: "hello from attach mode",
      },
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );

    expect(chunks.join("")).toBe("Fake response for: hello from attach mode");
    expect(result.output_text).toBe(
      "Fake response for: hello from attach mode",
    );
    expect(result.conversation_id).toMatch(/^conv-/);
    expect(result.message_id).toMatch(/^msg-/);
    expect(result.current_url).toContain(`/c/${result.conversation_id}`);
    expect(conversationIdFromUrl(result.current_url)).toBe(
      result.conversation_id,
    );

    await adapter.shutdown();
  });

  it("starts a fresh conversation for new ask requests in attach mode", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, { variant: "live" });
    const adapter = await harness.createAdapterForMode("attach", targetUrl);
    await adapter.open({ targetUrl, headless: true });

    const first = await adapter.ask(
      {
        jobId: "job-ask-attach-first-conversation",
        prompt: "start attach thread",
      },
      () => {},
    );
    const fresh = await adapter.ask(
      {
        jobId: "job-ask-attach-fresh-conversation",
        prompt: "start attach fresh thread",
        newConversation: true,
      },
      () => {},
    );

    expect(fresh.conversation_id).not.toBe(first.conversation_id);
    expect(conversationIdFromUrl(fresh.current_url)).toBe(
      fresh.conversation_id,
    );
    await expect(
      adapter
        .getManagedPageForTest()!
        .evaluate(() => document.body.dataset.newChatShortcut),
    ).resolves.toBe("1");

    await adapter.shutdown();
  });

  it("does not submit new ask requests into a stale root conversation", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, { variant: "live" });
    const adapter = await harness.createAdapterForMode("attach", targetUrl);
    await adapter.open({ targetUrl, headless: true });

    const first = await adapter.ask(
      {
        jobId: "job-ask-attach-stale-root-first",
        prompt: "start stale root thread",
      },
      () => {},
    );

    await adapter.getManagedPageForTest()!.evaluate(() => {
      const url = new URL(window.location.href);
      url.pathname = "/";
      window.history.replaceState({}, "", url);
      const staleTurn = document.createElement("div");
      staleTurn.setAttribute("data-testid", "conversation-turn-stale");
      staleTurn.textContent = "stale root turn";
      document.body.appendChild(staleTurn);
    });

    const fresh = await adapter.ask(
      {
        jobId: "job-ask-attach-stale-root-fresh",
        prompt: "start stale root fresh thread",
        newConversation: true,
      },
      () => {},
    );

    expect(fresh.conversation_id).not.toBe(first.conversation_id);
    expect(conversationIdFromUrl(fresh.current_url)).toBe(
      fresh.conversation_id,
    );
    await expect(
      adapter
        .getManagedPageForTest()!
        .evaluate(() => document.body.innerText.includes("stale root turn")),
    ).resolves.toBe(false);
    await expect(
      adapter
        .getManagedPageForTest()!
        .evaluate(() => document.body.dataset.newChatShortcut),
    ).resolves.toBe("1");

    await adapter.shutdown();
  });

  it("waits for a stabilized final ask snapshot in attach mode", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "done-early",
    });
    const adapter = await harness.createAdapterForMode("attach", targetUrl);
    const fixtureRoot = await harness.createWorkspace("browser");
    const fixture = path.join(fixtureRoot, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    await adapter.open({ targetUrl, headless: true });

    const chunks: string[] = [];
    const result = await adapter.ask(
      {
        jobId: "job-ask-done-early-attach",
        prompt: "Reply with exactly: ASK_OK",
        files: [fixture],
      },
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );

    expect(result.output_text).toBe(
      "Fake response for: Reply with exactly: ASK_OK",
    );
    expect(chunks.join("")).toBe(result.output_text);

    await adapter.shutdown();
  });

  it("fails fast before composer mutation when attach preflight network checks fail", async () => {
    const server = await harness.startServer();
    const attached = await harness.startAttachableBrowser(server.url);

    const root = await harness.createWorkspace("attach-network-preflight");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
        connectionMode: "attach_first",
        debugEndpoint: attached.endpoint,
      }),
    );
    await adapter.open({ targetUrl: server.url, headless: true });
    await stopFakeChatGptServer(server.process);

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-attach-network-preflight-fail",
          prompt: "should never reach composer",
        },
        () => {},
      ),
    ).rejects.toThrow(/provider\/network-unavailable/i);

    const pageState = await withAttachedBrowser(
      attached.endpoint,
      async (browser) => {
        const page = browser
          .contexts()
          .flatMap((context) => context.pages())
          .find((candidate) => candidate.url().startsWith(server.url));
        if (!page) {
          throw new Error("expected attached target page");
        }
        return page.evaluate(() => ({
          composerValue:
            (
              document.querySelector(
                "#prompt-textarea",
              ) as HTMLTextAreaElement | null
            )?.value ?? "",
          lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
        }));
      },
    );

    expect(pageState.composerValue).toBe("");
    expect(pageState.lastSubmissionMethod).toBe("");
    await adapter.shutdown();
  });

  it("fails with a capability-missing error in attach mode when search mode is unavailable", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, { missing: ["search"] });
    const adapter = await harness.createAdapterForMode("attach", targetUrl);
    await adapter.open({ targetUrl, headless: true });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-missing-search-attach",
          prompt: "search in attach mode",
          mode: "search",
        },
        () => {},
      ),
    ).rejects.toThrow(/capability missing: search mode/i);

    await adapter.shutdown();
  });

  it("fails cleanly when the harness never completes streaming in attach mode", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, { scenario: "timeout" });
    const adapter = await harness.createAdapterForMode("attach", targetUrl);
    const bundleRoot = await harness.createWorkspace("browser");
    await adapter.open({ targetUrl, headless: true });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-timeout-attach",
          prompt: "never finish in attach mode",
          timeoutMs: 600,
        },
        () => {},
      ),
    ).rejects.toThrow("timed out");

    const diagnostics = await adapter.diagnostics(
      path.join(bundleRoot, "bundle"),
    );
    expect(diagnostics.screenshot_path).toContain("failure.png");
    expect(diagnostics.dom_snapshot).toContain("dom_text");

    await adapter.shutdown();
  });
});
