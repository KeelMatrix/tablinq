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

function mermaidPreviewSvg(labels: string[]): string {
  return encodeURIComponent(`
    <svg xmlns="http://www.w3.org/2000/svg">
      ${labels.map((label) => `<text>${label}</text>`).join("")}
    </svg>
  `);
}

function mermaidPreviewOnlyMarkup(labels: string[]): string {
  return `
    <pre>
      <button aria-label="Code" type="button">Code</button>
      <button aria-label="Preview" type="button">Preview</button>
      <img alt="Mermaid preview" src="data:image/svg+xml,${mermaidPreviewSvg(labels)}">
    </pre>`;
}

function mermaidPreviewWithSourceMarkup(source: string): string {
  return `
    <pre data-testid="mermaid-widget">
      <button aria-label="Code" type="button">Code</button>
      <button aria-label="Preview" type="button">Preview</button>
      <img alt="Mermaid preview" src="data:image/svg+xml,${mermaidPreviewSvg([
        "Preview",
        "Only",
      ])}">
      <textarea data-language="mermaid" aria-label="Mermaid source" hidden>${source}</textarea>
    </pre>`;
}

function svgPreviewOnlyMarkup(labels: string[]): string {
  return `
    <pre data-testid="svg-widget" data-language="svg">
      <div>SVG</div>
      <button aria-label="Code" type="button">Code</button>
      <button aria-label="Preview" type="button">Preview</button>
      <img alt="SVG preview" src="data:image/svg+xml,${mermaidPreviewSvg(labels)}">
    </pre>`;
}

function svgPreviewWithSourceMarkup(source: string): string {
  return `
    <pre data-testid="svg-widget" data-language="svg">
      <div>SVG</div>
      <button aria-label="Code" type="button">Code</button>
      <button aria-label="Preview" type="button">Preview</button>
      <img alt="SVG preview" src="data:image/svg+xml,${mermaidPreviewSvg(["Preview"])}">
      <code data-language="svg" hidden>${source}</code>
    </pre>`;
}

async function readInsertedAskMarkdown(markdownHtml: string): Promise<string> {
  const { adapter, dom, main } = await createAskRuntimeDom();
  const baseline = adapter.readAskBaseline();
  main.insertAdjacentHTML(
    "beforeend",
    `<section data-testid="conversation-turn-markdown">
      <h4 class="sr-only">ChatGPT said:</h4>
      <div data-message-author-role="assistant" data-message-id="markdown-msg">
        <div class="markdown">${markdownHtml}</div>
      </div>
    </section>`,
  );
  dom.window.document.body.dataset.streamState = "done";

  const snapshot = adapter.readAskSnapshot({ baseline });
  dom.window.close();
  return snapshot.text;
}

async function readMaterializedAskMarkdown(
  markdownHtml: string,
  configure: (document: Document) => void,
): Promise<string> {
  const { adapter, dom, main } = await createAskRuntimeDom();
  const baseline = adapter.readAskBaseline();
  main.insertAdjacentHTML(
    "beforeend",
    `<section data-testid="conversation-turn-markdown">
      <h4 class="sr-only">ChatGPT said:</h4>
      <div data-message-author-role="assistant" data-message-id="markdown-msg">
        <div class="markdown">${markdownHtml}</div>
      </div>
    </section>`,
  );
  configure(dom.window.document);
  dom.window.document.body.dataset.streamState = "done";

  await adapter.materializeAskCodePreviewSources({ baseline });
  const snapshot = adapter.readAskSnapshot({ baseline });
  dom.window.close();
  return snapshot.text;
}

function installWidgetCodeViewSource(
  document: Document,
  testId: string,
  language: string,
  source: string,
) {
  const widget = document.querySelector(`[data-testid='${testId}']`);
  const code = widget?.querySelector("button[aria-label='Code']");
  code?.addEventListener("click", () => {
    const sourceElement = document.createElement("code");
    sourceElement.dataset.language = language;
    sourceElement.textContent = source;
    widget?.appendChild(sourceElement);
  });
}

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
  it("prefers hidden Mermaid source over preview SVG labels", async () => {
    const source = "graph TD\n  A[Start] --> B[Done]";
    const text = await readInsertedAskMarkdown(
      mermaidPreviewWithSourceMarkup(source),
    );

    expect(text).toBe(`\`\`\`mermaid\n${source}\n\`\`\``);
    expect(text).not.toContain("Mermaid diagram preview");
    expect(text).not.toContain("Preview; Only");
  });

  it("keeps rendered Mermaid pre code as a fenced code block", async () => {
    const source = "sequenceDiagram\n  Alice->>Bob: Hello";
    const text = await readInsertedAskMarkdown(
      `<pre data-language="mermaid"><code>${source}</code></pre>`,
    );

    expect(text).toBe(`\`\`\`mermaid\n${source}\n\`\`\``);
  });

  it("keeps the Mermaid preview fallback when no source code is available", async () => {
    const text = await readInsertedAskMarkdown(
      mermaidPreviewOnlyMarkup(["Start", "Process", "Done"]),
    );

    expect(text).toBe("Mermaid diagram preview: Start; Process; Done.");
  });

  it("treats Mermaid loading labels as preview chrome, not fenced source", async () => {
    const text = await readInsertedAskMarkdown(`
      <pre data-testid="mermaid-widget" data-language="mermaid">
        <div>Mermaid</div>
        <button aria-label="Code" type="button">Code</button>
        <button aria-label="Preview" type="button">Preview</button>
        <div>Loading diagram...</div>
        <canvas aria-label="Mermaid preview"></canvas>
      </pre>
    `);

    expect(text).toBe("");
    expect(text).not.toContain("```mermaid");
    expect(text).not.toContain("Loading diagram");
  });

  it("withholds transient Mermaid preview state until stable source appears", async () => {
    const { adapter, dom, main } = await createAskRuntimeDom();
    const baseline = adapter.readAskBaseline();
    main.insertAdjacentHTML(
      "beforeend",
      `<section data-testid="conversation-turn-markdown">
        <h4 class="sr-only">ChatGPT said:</h4>
        <div data-message-author-role="assistant" data-message-id="markdown-msg">
          <div class="markdown">
            <pre data-testid="mermaid-widget" data-language="mermaid">
              <div>Mermaid</div>
              <button aria-label="Code" type="button">Code</button>
              <button aria-label="Preview" type="button">Preview</button>
              <div>Loading diagram...</div>
              <canvas aria-label="Mermaid preview"></canvas>
            </pre>
          </div>
        </div>
      </section>`,
    );
    dom.window.document.body.dataset.streamState = "done";

    const first = adapter.readAskSnapshot({ baseline });
    const second = adapter.readAskSnapshot({ baseline });
    expect(first.text).toBe("");
    expect(second.text).toBe("");
    expect(first.hasTransientWidgets).toBe(true);

    const widget = dom.window.document.querySelector(
      "[data-testid='mermaid-widget']",
    );
    if (widget) {
      widget.innerHTML = `<code data-language="mermaid">graph TD\n  A --> B</code>`;
    }
    const third = adapter.readAskSnapshot({ baseline });

    expect(third.text).toBe("```mermaid\ngraph TD\n  A --> B\n```");
    expect(third.hasTransientWidgets).toBe(false);
    dom.window.close();
  });

  it("withholds transient SVG preview state until stable source appears", async () => {
    const { adapter, dom, main } = await createAskRuntimeDom();
    const baseline = adapter.readAskBaseline();
    main.insertAdjacentHTML(
      "beforeend",
      `<section data-testid="conversation-turn-markdown">
        <h4 class="sr-only">ChatGPT said:</h4>
        <div data-message-author-role="assistant" data-message-id="markdown-msg">
          <div class="markdown">
            <pre data-testid="svg-widget" data-language="svg">
              <div>SVG</div>
              <button aria-label="Code" type="button">Code</button>
              <button aria-label="Preview" type="button">Preview</button>
              <div>Rendering...</div>
              <canvas aria-label="SVG preview"></canvas>
            </pre>
          </div>
        </div>
      </section>`,
    );
    dom.window.document.body.dataset.streamState = "done";

    const first = adapter.readAskSnapshot({ baseline });
    expect(first.text).toBe("");
    expect(first.hasTransientWidgets).toBe(true);

    const svgSource = '<svg xmlns="http://www.w3.org/2000/svg"></svg>';
    const widget = dom.window.document.querySelector(
      "[data-testid='svg-widget']",
    );
    if (widget) {
      const code = dom.window.document.createElement("code");
      code.dataset.language = "svg";
      code.textContent = svgSource;
      widget.replaceChildren(code);
    }
    const stable = adapter.readAskSnapshot({ baseline });

    expect(stable.text).toBe(`\`\`\`svg\n${svgSource}\n\`\`\``);
    expect(stable.hasTransientWidgets).toBe(false);
    dom.window.close();
  });

  it("preserves prose while withholding a transient widget", async () => {
    const { adapter, dom, main } = await createAskRuntimeDom();
    const baseline = adapter.readAskBaseline();
    main.insertAdjacentHTML(
      "beforeend",
      `<section data-testid="conversation-turn-markdown">
        <h4 class="sr-only">ChatGPT said:</h4>
        <div data-message-author-role="assistant" data-message-id="markdown-msg">
          <div class="markdown">
            <p>Here is the diagram:</p>
            <pre data-testid="mermaid-widget" data-language="mermaid">
              <div>Generating...</div>
              <button aria-label="Code" type="button">Code</button>
              <button aria-label="Preview" type="button">Preview</button>
              <canvas aria-label="Mermaid preview"></canvas>
            </pre>
          </div>
        </div>
      </section>`,
    );
    dom.window.document.body.dataset.streamState = "done";

    const snapshot = adapter.readAskSnapshot({ baseline });

    expect(snapshot.text).toBe("Here is the diagram:");
    expect(snapshot.hasTransientWidgets).toBe(true);
    dom.window.close();
  });

  it("does not serialize SVG preview widgets as Mermaid diagrams", async () => {
    const text = await readInsertedAskMarkdown(
      svgPreviewOnlyMarkup(["SOURCE_SHOULD_SURVIVE"]),
    );

    expect(text).toBe("SVG preview: SOURCE_SHOULD_SURVIVE.");
    expect(text).not.toContain("Mermaid diagram preview");
  });

  it("prefers SVG source over SVG preview labels", async () => {
    const source =
      "&lt;svg xmlns=&quot;http://www.w3.org/2000/svg&quot;&gt;&lt;text&gt;SOURCE_SHOULD_SURVIVE&lt;/text&gt;&lt;/svg&gt;";
    const text = await readInsertedAskMarkdown(
      svgPreviewWithSourceMarkup(source),
    );

    expect(text).toBe(
      '```svg\n<svg xmlns="http://www.w3.org/2000/svg"><text>SOURCE_SHOULD_SURVIVE</text></svg>\n```',
    );
    expect(text).not.toContain("SVG preview");
    expect(text).not.toContain("Mermaid diagram preview");
  });

  it("materializes Mermaid source from the widget Code view before final extraction", async () => {
    const source = "graph TD\n  A[Start] --> B[Done]";
    const text = await readMaterializedAskMarkdown(
      `
      <pre data-testid="mermaid-widget" data-language="mermaid">
        <div>Mermaid</div>
        <button aria-label="Code" type="button">Code</button>
        <button aria-label="Preview" type="button">Preview</button>
        <div>Loading diagram...</div>
        <canvas aria-label="Mermaid preview"></canvas>
      </pre>
    `,
      (document) => {
        installWidgetCodeViewSource(
          document,
          "mermaid-widget",
          "mermaid",
          source,
        );
      },
    );

    expect(text).toBe(`\`\`\`mermaid\n${source}\n\`\`\``);
    expect(text).not.toContain("Mermaid diagram preview");
    expect(text).not.toContain("Loading diagram");
  });

  it("materializes Mermaid source from a toggle-only live widget", async () => {
    const source = "graph TD\n  Login --> Session";
    const text = await readMaterializedAskMarkdown(
      `
      <pre data-testid="mermaid-widget">
        <div>Mermaid</div>
        <button aria-label="Code" aria-pressed="false" type="button">Code</button>
        <button aria-label="Preview" aria-pressed="true" type="button">Preview</button>
      </pre>
    `,
      (document) => {
        installWidgetCodeViewSource(
          document,
          "mermaid-widget",
          "mermaid",
          source,
        );
      },
    );

    expect(text).toBe(`\`\`\`mermaid\n${source}\n\`\`\``);
    expect(text).not.toContain("Mermaid diagram preview");
  });

  it("preserves CodeMirror line breaks when materializing widget source", async () => {
    const text = await readMaterializedAskMarkdown(
      `
      <pre data-testid="mermaid-widget">
        <div>Mermaid</div>
        <button aria-label="Code" aria-pressed="false" type="button">Code</button>
        <button aria-label="Preview" aria-pressed="true" type="button">Preview</button>
      </pre>
    `,
      (document) => {
        const widget = document.querySelector("[data-testid='mermaid-widget']");
        const code = widget?.querySelector("button[aria-label='Code']");
        code?.addEventListener("click", () => {
          widget?.insertAdjacentHTML(
            "beforeend",
            `<pre class="cm-content"><code><span>flowchart TD</span><br><span>  A --> B</span></code></pre>`,
          );
        });
      },
    );

    expect(text).toBe("```mermaid\nflowchart TD\n  A --> B\n```");
  });

  it("materializes SVG source from the widget Code view before final extraction", async () => {
    const source =
      '<svg xmlns="http://www.w3.org/2000/svg"><text>Source survives</text></svg>';
    const text = await readMaterializedAskMarkdown(
      `
      <pre data-testid="svg-widget" data-language="svg">
        <div>SVG</div>
        <button aria-label="Code" type="button">Code</button>
        <button aria-label="Preview" type="button">Preview</button>
        <img alt="SVG preview" src="data:image/svg+xml,${mermaidPreviewSvg(["Preview label"])}">
      </pre>
    `,
      (document) => {
        installWidgetCodeViewSource(document, "svg-widget", "svg", source);
      },
    );

    expect(text).toBe(`\`\`\`svg\n${source}\n\`\`\``);
    expect(text).not.toContain("Mermaid diagram preview");
    expect(text).not.toContain("SVG preview");
  });

  it("materializes mixed code-preview widget sources in answer order", async () => {
    const mermaidSource = "graph LR\n  A --> B";
    const svgSource =
      '<svg xmlns="http://www.w3.org/2000/svg"><text>Diagram</text></svg>';
    const text = await readMaterializedAskMarkdown(
      `
      <pre data-language="json"><code>{"ok": true}</code></pre>
      <pre data-language="yaml"><code>ok: true</code></pre>
      <pre data-language="csv"><code>name,value\nalpha,1</code></pre>
      <pre data-testid="mermaid-widget" data-language="mermaid">
        <div>Mermaid</div>
        <button aria-label="Code" type="button">Code</button>
        <button aria-label="Preview" type="button">Preview</button>
        <div>Loading diagram...</div>
        <canvas aria-label="Mermaid preview"></canvas>
      </pre>
      <pre data-testid="svg-widget" data-language="svg">
        <div>SVG</div>
        <button aria-label="Code" type="button">Code</button>
        <button aria-label="Preview" type="button">Preview</button>
        <img alt="SVG preview" src="data:image/svg+xml,${mermaidPreviewSvg(["Preview label"])}">
      </pre>
      <pre data-language="html"><code>&lt;main&gt;ok&lt;/main&gt;</code></pre>
    `,
      (document) => {
        installWidgetCodeViewSource(
          document,
          "mermaid-widget",
          "mermaid",
          mermaidSource,
        );
        installWidgetCodeViewSource(document, "svg-widget", "svg", svgSource);
      },
    );

    expect(text).toBe(
      `\`\`\`json\n{"ok": true}\n\`\`\`\n\n` +
        `\`\`\`yaml\nok: true\n\`\`\`\n\n` +
        `\`\`\`csv\nname,value\nalpha,1\n\`\`\`\n\n` +
        `\`\`\`mermaid\n${mermaidSource}\n\`\`\`\n\n` +
        `\`\`\`svg\n${svgSource}\n\`\`\`\n\n` +
        "```html\n<main>ok</main>\n```",
    );
    expect(text).not.toContain("Mermaid diagram preview");
    expect(text).not.toContain("Loading diagram");
  });

  it("preserves mixed fenced blocks and Mermaid source in order", async () => {
    const mermaidSource = "graph LR\n  A --> B";
    const text = await readInsertedAskMarkdown(`
      <pre data-language="json"><code>{"ok": true}</code></pre>
      <pre data-language="yaml"><code>ok: true</code></pre>
      <pre data-language="csv"><code>name,value\nalpha,1</code></pre>
      <pre data-language="html"><code>&lt;main&gt;ok&lt;/main&gt;</code></pre>
      ${mermaidPreviewWithSourceMarkup(mermaidSource)}
    `);

    expect(text).toBe(
      `\`\`\`json\n{"ok": true}\n\`\`\`\n\n` +
        `\`\`\`yaml\nok: true\n\`\`\`\n\n` +
        `\`\`\`csv\nname,value\nalpha,1\n\`\`\`\n\n` +
        `\`\`\`html\n<main>ok</main>\n\`\`\`\n\n` +
        `\`\`\`mermaid\n${mermaidSource}\n\`\`\``,
    );
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
