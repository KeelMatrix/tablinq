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
import { uploadAttachments } from "../src/pageRuntime/uploads";
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
  liveCanvasArtifactText,
  liveStyleAnswerMessage,
  liveStyleCanvasArtifactTurn,
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

describe("browser adapter ask: uploads and artifacts", () => {
  it("suppresses thought artifact messages and streams the actual reply", async () => {
    const server = await harness.startServer();
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "thinking-artifact-messages",
    });
    const { adapter } = await openTrackedManagedAdapter(harness, targetUrl);

    const stream = captureDeltaChunks();
    const result = await adapter.ask(
      {
        jobId: "job-ask-thinking-artifact-messages",
        prompt: "reason carefully about this tradeoff",
      },
      stream.onChunk,
    );

    expect(result.output_text).toBe(
      "Fake response for: reason carefully about this tradeoff",
    );
    expect(stream.text()).toBe(result.output_text);
    expect(stream.chunks).toEqual([result.output_text]);
    expect(stream.text()).not.toContain("I’m reconstructing the prior context");
    expect(stream.text()).not.toContain("The comparison is narrowing");

    await adapter.shutdown();
  });

  it("ignores a live-style thought artifact before the answer starts", async () => {
    const { adapter, dom, main } = await createAskRuntimeDom();
    const baseline = adapter.readAskBaseline();
    main.insertAdjacentHTML("beforeend", liveStyleReasoningTurn());
    dom.window.document.body.dataset.streamState = "running";

    const snapshot = adapter.readAskSnapshot({ baseline });

    expect(snapshot.text).toBe("");
    expect(snapshot.text).not.toContain(liveThoughtArtifactText);
    dom.window.close();
  });

  it("reads only the live-style actual answer after a thought artifact", async () => {
    const { adapter, dom, main } = await createAskRuntimeDom();
    const baseline = adapter.readAskBaseline();
    main.insertAdjacentHTML(
      "beforeend",
      liveStyleReasoningTurn(liveActualAnswerText),
    );
    dom.window.document.body.dataset.streamState = "done";

    const snapshot = adapter.readAskSnapshot({ baseline });

    expect(snapshot.messageId).toBe("actual-answer-msg");
    expect(snapshot.text).toBe(liveActualAnswerText);
    expect(snapshot.text).not.toContain(liveThoughtArtifactText);
    expect(snapshot.text).not.toBe(
      `${liveActualAnswerText}\n\n${liveActualAnswerText}`,
    );
    dom.window.close();
  });

  it("keeps useful pre-thought canvas text with the following answer", async () => {
    const { adapter, dom, main } = await createAskRuntimeDom();
    const baseline = adapter.readAskBaseline();
    const answerText =
      "Create this folder in the repo: scripts/smoke/local-gateway/";
    main.insertAdjacentHTML(
      "beforeend",
      liveStyleCanvasArtifactTurn(answerText),
    );
    dom.window.document.body.dataset.streamState = "done";

    const snapshot = adapter.readAskSnapshot({ baseline });

    expect(snapshot.messageId).toBe("actual-answer-msg");
    expect(snapshot.text).toContain(liveCanvasArtifactText);
    expect(snapshot.text).toContain(answerText);
    expect(snapshot.text).not.toContain(liveThoughtArtifactText);
    expect(snapshot.text).not.toContain("Thought for 23s");
    expect(snapshot.text).not.toContain("Copy\n\nEdit\n\nDownload");
    dom.window.close();
  });

  it("does not emit pre-thought canvas text before a following answer exists", async () => {
    const { adapter, dom, main } = await createAskRuntimeDom();
    const baseline = adapter.readAskBaseline();
    main.insertAdjacentHTML("beforeend", liveStyleCanvasArtifactTurn());
    dom.window.document.body.dataset.streamState = "running";

    const snapshot = adapter.readAskSnapshot({ baseline });

    expect(snapshot.text).toBe("");
    expect(snapshot.text).not.toContain(liveCanvasArtifactText);
    expect(snapshot.text).not.toContain(liveThoughtArtifactText);
    dom.window.close();
  });

  it("does not prepend provider labels that appear before the thought divider", async () => {
    const { adapter, dom, main } = await createAskRuntimeDom();
    const baseline = adapter.readAskBaseline();
    const answerText = "The useful answer starts here.";
    main.insertAdjacentHTML(
      "beforeend",
      `
      <section data-testid="conversation-turn-4">
        <div class="flex max-w-full flex-col gap-4 grow">
          <div data-message-author-role="assistant" data-message-id="reasoning-artifact-msg">
            <div class="markdown"><p>${liveThoughtArtifactText}</p></div>
          </div>
          <div class="provider-labels">
            <span>Searching</span>
            <span>Sources</span>
            <button type="button">Copy</button>
          </div>
          <div class="flex items-center justify-between">
            <button type="button">Thought for 11s</button>
          </div>
          ${liveStyleAnswerMessage(answerText)}
        </div>
      </section>`,
    );
    dom.window.document.body.dataset.streamState = "done";

    const snapshot = adapter.readAskSnapshot({ baseline });

    expect(snapshot.text).toBe(answerText);
    expect(snapshot.text).not.toContain("Searching");
    expect(snapshot.text).not.toContain("Sources");
    expect(snapshot.text).not.toContain(liveThoughtArtifactText);
    dom.window.close();
  });

  it("uploads files and persists generated artifacts", async () => {
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
        defaultTargetUrl: server.url,
      }),
    );
    await adapter.open({ targetUrl: server.url, headless: true });

    const result = await adapter.ask(
      {
        jobId: "job-ask-upload-artifact",
        prompt: "summarize uploads",
        files: [firstFile, secondFile],
      },
      () => {},
    );

    expect(result.artifacts).toHaveLength(1);
    expect(result.artifacts[0]!.file_name).toContain("brief-summary");
    expect(result.artifacts[0]!.content_type).toBe("text/plain; charset=utf-8");
    expect(result.artifacts[0]!.size_bytes).toBeGreaterThan(0);
    const downloaded = await stat(result.artifacts[0]!.storage_path);
    expect(downloaded.size).toBeGreaterThan(0);
    const artifactBody = await readFile(
      result.artifacts[0]!.storage_path,
      "utf8",
    );
    expect(artifactBody).toContain("Uploads: brief.txt, notes.csv");

    await adapter.shutdown();
  });

  it("fails with an upload error when files are requested without an upload control", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: withHarnessUrl(server.url, { missing: ["upload"] }),
      }),
    );
    await adapter.open({
      targetUrl: withHarnessUrl(server.url, { missing: ["upload"] }),
      headless: true,
    });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-missing-upload",
          prompt: "summarize uploads",
          files: [fixture],
        },
        () => {},
      ),
    ).rejects.toThrow(/upload failed/i);

    await adapter.shutdown();
  });

  it("waits for a delayed Deep Research upload input before attaching files", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const { adapter } = await openTrackedManagedAdapter(harness, server.url);
    const page = adapter.getManagedPageForTest()!;

    await page.setContent(`
      <main>
        <form data-testid="composer-root">
          <textarea id="prompt-textarea" aria-label="Message"></textarea>
          <button id="composer-plus-btn" type="button" aria-label="Add files and more"></button>
          <div data-slot="upload-list"></div>
        </form>
        <script>
          document.addEventListener("change", (event) => {
            const input = event.target;
            if (!(input instanceof HTMLInputElement)) {
              return;
            }
            const list = document.querySelector("[data-slot='upload-list']");
            list.innerHTML = Array.from(input.files || [])
              .map((file) => '<span data-slot="uploaded-file">' + file.name + '</span>')
              .join("");
          });
          setTimeout(() => {
            const input = document.createElement("input");
            input.id = "upload-files";
            input.type = "file";
            input.multiple = true;
            input.style.position = "absolute";
            input.style.width = "1px";
            input.style.height = "1px";
            input.style.clipPath = "inset(50%)";
            document.querySelector("form").append(input);
          }, 250);
        </script>
    `);

    await uploadAttachments(page, [fixture], null);

    await expect(
      page.locator("[data-slot='uploaded-file']").textContent(),
    ).resolves.toBe("brief.txt");
    await adapter.shutdown();
  });

  it("ignores a stale visible send control ahead of the active composer after upload acknowledgment", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "submit-upload-stale-recover",
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
        jobId: "job-ask-submit-upload-stale-recover",
        prompt: "repeat attached file",
        files: [fixture],
        timeoutMs: 10_000,
      },
      () => {},
    );

    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
      staleSubmitClicks: document.body.dataset.staleSubmitClicks,
      keyboardSubmitReady: document.body.dataset.keyboardSubmitReady,
    }));

    expect(result.output_text).toBe("Fake response for: repeat attached file");
    expect(state).toEqual({
      lastSubmissionMethod: "button",
      staleSubmitClicks: "0",
      keyboardSubmitReady: "false",
    });

    await adapter.shutdown();
  });

  it("waits for real submit readiness after upload acknowledgment", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "upload-processing-delay",
    });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    const startedAt = Date.now();
    const result = await adapter.ask(
      {
        jobId: "job-ask-upload-processing-delay",
        prompt: "repeat attached file after processing",
        files: [fixture],
        timeoutMs: 10_000,
      },
      () => {},
    );
    const elapsedMs = Date.now() - startedAt;
    const state = await adapter.getManagedPageForTest()!.evaluate(() => ({
      uploadProcessing: document.body.dataset.uploadProcessing,
      lastSubmissionMethod: document.body.dataset.lastSubmissionMethod,
    }));

    expect(result.output_text).toBe(
      "Fake response for: repeat attached file after processing",
    );
    expect(elapsedMs).toBeGreaterThanOrEqual(1_800);
    expect(state).toEqual({
      uploadProcessing: "false",
      lastSubmissionMethod: "button",
    });

    await adapter.shutdown();
  });

  it("fails clearly when upload acknowledgment never becomes send-ready", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "upload-processing-stuck",
    });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-upload-processing-stuck",
          prompt: "repeat attached file",
          files: [fixture],
          timeoutMs: 1_500,
        },
        () => {},
      ),
    ).rejects.toThrow(/attachments are still processing/i);
    await expect(
      adapter.getManagedPageForTest()!.evaluate(() => ({
        composerValue:
          (document.querySelector("#prompt-textarea") as HTMLTextAreaElement)
            ?.value ?? "",
      })),
    ).resolves.toEqual({
      composerValue: "",
    });

    await adapter.shutdown();
  });

  it("rejects files larger than 20 MiB before upload", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "oversize.bin");
    await writeFile(fixture, Buffer.alloc(MAX_UPLOAD_FILE_BYTES + 1));
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: server.url,
      }),
    );
    await adapter.open({ targetUrl: server.url, headless: true });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-oversize-upload",
          prompt: "repeat attached file",
          files: [fixture],
        },
        () => {},
      ),
    ).rejects.toThrow(/file exceeds 20 MiB/i);

    await adapter.shutdown();
  });

  it("fails clearly when submit never becomes ready after upload acknowledgment", async () => {
    const server = await harness.startServer();
    const root = await harness.createWorkspace("browser");
    const fixture = path.join(root, "brief.txt");
    await writeFile(fixture, "brief body", "utf8");
    const targetUrl = withHarnessUrl(server.url, {
      variant: "live",
      scenario: "submit-never-ready",
    });
    const adapter = harness.trackAdapter(
      new BrowserAdapter({
        profileDir: path.join(root, "profile"),
        downloadsDir: path.join(root, "downloads"),
        defaultTargetUrl: targetUrl,
      }),
    );
    await adapter.open({ targetUrl, headless: true });

    await expect(
      adapter.ask(
        {
          jobId: "job-ask-submit-upload-trap-fail",
          prompt: "repeat attached file",
          files: [fixture],
          timeoutMs: 1_500,
        },
        () => {},
      ),
    ).rejects.toThrow(/submit never became ready/i);
    await expect(
      adapter.getManagedPageForTest()!.evaluate(() => ({
        composerValue:
          (document.querySelector("#prompt-textarea") as HTMLTextAreaElement)
            ?.value ?? "",
      })),
    ).resolves.toEqual({
      composerValue: "",
    });

    await adapter.shutdown();
  });
});
