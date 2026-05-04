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

describe("browser adapter ask: error paths", () => {
  it("refuses asks when the managed page is a signed-out local login screen", async () => {
    const server = await harness.startServer();
    const { adapter } = await openTrackedManagedAdapter(
      harness,
      `${server.url}/login-expired.html`,
    );

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-auth-expired",
          prompt: "should refuse while signed out",
        },
        () => {},
      ),
    ).rejects.toThrow("auth required");

    await adapter.shutdown();
  });

  it("fails cleanly when the harness never completes streaming", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, { scenario: "timeout" }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { scenario: "timeout" }),
      headless: true,
    });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-timeout",
          prompt: "never finish",
          timeoutMs: 600,
        },
        () => {},
      ),
    ).rejects.toThrow("timed out");

    const diagnostics = await adapter.diagnostics(path.join(root, "bundle"));
    expect(diagnostics.screenshot_path).toContain("failure.png");
    expect(diagnostics.dom_snapshot).toContain("dom_text");
    expect(diagnostics.aria_snapshot).toContain("role");

    await adapter.shutdown();
  });

  it("reloads once before failing an ask timeout and returns a response if it appears", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const targetUrl = withHarnessUrl(server.url, {
      scenario: "timeout-reload-success",
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
        jobId: "job-ask-timeout-reload-success",
        prompt: "finish after timeout reload",
        timeoutMs: 600,
      },
      () => {},
    );

    expect(result.output_text).toContain(
      "Fake response for: finish after timeout reload",
    );

    await adapter.shutdown();
  });

  it("classifies provider generation failures without clicking Retry", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "provider-generation-failure-twice",
    });
    const root = await harness.createWorkspace("browser");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    let error: any = null;
    try {
      await adapter.ask(
        {
          jobId: "job-ask-provider-generation-failure",
          prompt: "reason carefully about this tradeoff",
        },
        () => {},
      );
    } catch (caught) {
      error = caught;
    }

    expect(error).toBeTruthy();
    expect(error.code).toBe("provider_generation_failed_nonretryable");
    expect(error.retryVisible).toBe(true);
    expect(providerPressureCooldownMs()).toBeGreaterThan(0);
    expect(
      await adapter.getManagedPageForTest()!.evaluate(() => ({
        askFailureCode: document.body.dataset.askFailureCode ?? null,
        retryClicks: document.body.dataset.retryClicks ?? "0",
      })),
    ).toEqual({
      askFailureCode: "provider_generation_failed_nonretryable",
      retryClicks: "0",
    });

    await adapter.shutdown();
  });

  it("refuses to open a missing provider conversation", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(`${server.url}/c/provider-missing`, {
      variant: "live",
      scenario: "conversation-load-failure-delayed",
    });
    const { adapter } = await openTrackedManagedAdapter(harness, server.url);

    let error: any = null;
    try {
      await adapter.open({ targetUrl, headless: true });
    } catch (caught) {
      error = caught;
    }

    expect(error).toBeTruthy();
    expect(error.code).toBe("provider_conversation_load_failed");
    expect(error.message).toContain(
      "ChatGPT did not load conversation provider-missing",
    );

    await adapter.shutdown();
  });

  it("refuses conversation asks before composer mutation when the provider conversation cannot load", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "conversation-load-failure",
    });
    const adapter = await harness.createAdapterForMode("attach", targetUrl);
    await adapter.open({ targetUrl, headless: true });

    let error: any = null;
    try {
      await adapter.ask(
        {
          jobId: "job-ask-provider-conversation-load-failure",
          prompt: "should not be submitted",
          conversationId: "provider-missing",
        },
        () => {},
      );
    } catch (caught) {
      error = caught;
    }

    expect(error).toBeTruthy();
    expect(error.code).toBe("provider_conversation_load_failed");
    await expect(
      adapter.getManagedPageForTest()!.evaluate(() => ({
        composerValue:
          (document.querySelector("#prompt-textarea") as HTMLTextAreaElement)
            ?.value ?? "",
        lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
      })),
    ).resolves.toEqual({
      composerValue: "",
      lastSubmissionMethod: "",
    });

    await adapter.shutdown();
  });
});
