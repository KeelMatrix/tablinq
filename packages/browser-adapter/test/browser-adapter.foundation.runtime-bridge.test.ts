import { mkdir, readFile, stat, writeFile } from "node:fs/promises";
import path from "node:path";
import { JSDOM } from "jsdom";
import {
  BrowserAdapter,
  adaptivePollIntervalMs,
  chooseUniqueArtifactPath,
  conversationIdFromUrl,
  deepResearchStartDelayMs,
  detectBrowserExecutable,
  highRiskActionCooldownMs,
  interactionJitterMs,
  MAX_UPLOAD_FILE_BYTES,
  noteProviderPressure,
  noteProviderPressureFromText,
  pacingRandomValueFromSeed,
  providerPressureCooldownMs,
  resetSessionRhythmForTest,
  resolveConversationUrl,
  resolveBrowserLaunchConfig,
  uploadAckTimeoutMsForFiles,
  workspaceKeyFromUrl,
} from "../src/browserAdapter";
import { applyAskSnapshot } from "../src/execution/streaming";
import { loadPageAdapterSource } from "../src/assets";
import {
  scrollToBottomIfNeeded,
  waitForInteractionSurface,
  waitForQuietInteractionSurface,
} from "../src/pageRuntime";
import {
  createIsolatedTempDir,
  listAttachedPages,
  withAttachedBrowser,
  withHarnessUrl,
} from "./browser-adapter.integration.helpers";
import {
  expectHarnessCapabilityMatrix,
  expectManagedHarnessOpenSnapshot,
  openManagedHarnessAdapter,
  openTrackedManagedAdapter,
} from "./helpers/browserFlow";
import { installBrowserAdapterHarness } from "./helpers/browserAdapterHarness";

const harness = installBrowserAdapterHarness({ resetSessionRhythm: true });

const readAssistantMarkdownSnapshotText = (
  markdownHtml: string,
  streamState = "done",
) => {
  const runtimeSource = loadPageAdapterSource();
  const dom = new JSDOM("<!doctype html><html><body></body></html>", {
    url: "https://chatgpt.com/c/conv-markdown-math",
    runScripts: "outside-only",
  });
  try {
    const { window } = dom;
    const article = window.document.createElement("article");
    article.setAttribute("data-slot", "assistant-message");
    article.hidden = false;
    article.setAttribute("data-message-author-role", "assistant");
    article.setAttribute("data-message-id", "msg-markdown-math");
    article.innerHTML = `<div class="markdown">${markdownHtml}</div>`;
    window.document.body.append(article);
    window.document.body.dataset.conversationId = "conv-markdown-math";
    window.document.body.dataset.lastMessageId = "msg-markdown-math";
    window.document.body.dataset.streamState = streamState;

    const snapshot = window.eval(
      `${runtimeSource.replace(
        /^export\s+/gm,
        "",
      )}\ncreateTablinqPageAdapterRuntime().readAskSnapshot();`,
    ) as { text: string };
    return snapshot.text;
  } finally {
    dom.window.close();
  }
};

const katexHtml = (tex: string, visibleHtml = "VISIBLE TOKEN SOUP") =>
  `<span class="katex">
    <span class="katex-mathml" hidden>
      <math>
        <semantics>
          <mrow></mrow>
          <annotation encoding="application/x-tex">${tex}</annotation>
        </semantics>
      </math>
    </span>
    <span class="katex-html">${visibleHtml}</span>
  </span>`;

describe("browser adapter foundation: page runtime bridge behavior", () => {
  it("loads a self-contained page adapter runtime source", () => {
    const factory = new Function(
      `${loadPageAdapterSource()}\nreturn createTablinqPageAdapterRuntime;`,
    )();

    expect(typeof factory).toBe("function");
    expect(Object.keys(factory()).sort()).toEqual([
      "applyAskConfiguration",
      "attachmentsStillProcessing",
      "clickSubmit",
      "composerReady",
      "fillComposer",
      "readAskBaseline",
      "readAskSnapshot",
      "readResearchSnapshot",
      "readResearchSubmissionState",
      "readSubmitState",
      "researchSubmissionStarted",
      "submitReady",
      "submitStarted",
      "uploadAcknowledged",
    ]);
  });

  it("keeps the page runtime command facade aligned with composer, submit, capability, and message probes", async () => {
    const runtimeSource = loadPageAdapterSource();
    const dom = new JSDOM(
      `<!doctype html><html><body data-uploaded-files="brief.txt" data-stream-state="done">
        <main>
          <form data-testid="composer-root">
            <textarea id="prompt-textarea" aria-label="Message"></textarea>
            <button data-testid="send-button" aria-label="Send message" type="button"></button>
            <button
              id="composer-plus-btn"
              aria-label="Add files and more"
              aria-haspopup="menu"
              aria-controls="tools-menu"
              aria-expanded="false"
              type="button"
            ></button>
            <div id="tools-menu" role="menu" hidden>
              <button id="search-mode-option" role="menuitemradio" type="button">Web search</button>
            </div>
            <button id="search-chip" class="__composer-pill" aria-label="Search" type="button" hidden>Search</button>
          </form>
          <article data-slot="assistant-message" data-message-author-role="assistant" data-message-id="msg-command">
            <div class="markdown"><p>Final answer.</p></div>
          </article>
        </main>
      </body></html>`,
      {
        url: "https://chatgpt.com/c/conv-command",
        runScripts: "outside-only",
      },
    );
    const { window } = dom;
    const adapter = window.eval(
      `${runtimeSource.replace(
        /^export\s+/gm,
        "",
      )}\ncreateTablinqPageAdapterRuntime();`,
    );
    const composer = window.document.querySelector("#prompt-textarea");
    const tools = window.document.querySelector("#tools-menu");
    const plus = window.document.querySelector("#composer-plus-btn");
    const search = window.document.querySelector("#search-mode-option");
    const searchChip = window.document.querySelector("#search-chip");
    if (
      !(composer instanceof window.HTMLTextAreaElement) ||
      !(tools instanceof window.HTMLElement) ||
      !(plus instanceof window.HTMLElement) ||
      !(search instanceof window.HTMLElement) ||
      !(searchChip instanceof window.HTMLElement)
    ) {
      throw new Error("expected runtime test controls");
    }
    plus.addEventListener("click", () => {
      const expanded = plus.getAttribute("aria-expanded") === "true";
      plus.setAttribute("aria-expanded", expanded ? "false" : "true");
      tools.toggleAttribute("hidden", expanded);
      search.toggleAttribute("hidden", expanded);
    });
    search.addEventListener("click", () => {
      search.setAttribute("aria-checked", "true");
      tools.setAttribute("hidden", "");
      search.setAttribute("hidden", "");
      plus.setAttribute("aria-expanded", "false");
      searchChip.removeAttribute("hidden");
      searchChip.setAttribute("aria-label", "Search, click to remove");
    });

    expect(Object.keys(adapter).sort()).toEqual([
      "applyAskConfiguration",
      "attachmentsStillProcessing",
      "clickSubmit",
      "composerReady",
      "fillComposer",
      "readAskBaseline",
      "readAskSnapshot",
      "readResearchSnapshot",
      "readResearchSubmissionState",
      "readSubmitState",
      "researchSubmissionStarted",
      "submitReady",
      "submitStarted",
      "uploadAcknowledged",
    ]);
    expect(adapter.composerReady()).toBe(true);
    expect(adapter.readSubmitState()).toMatchObject({
      composerText: "",
      submitVisible: true,
      submitEnabled: true,
      submitBusy: false,
      keyboardReady: false,
      attachmentsProcessing: false,
      stopVisible: false,
      assistantMessageCount: 1,
      streamState: "done",
    });

    adapter.fillComposer({ prompt: "Hello command facade" });
    expect(composer.value).toBe("Hello command facade");
    expect(adapter.submitReady()).toBe(true);
    expect(adapter.uploadAcknowledged({ names: ["brief.txt"] })).toBe(true);

    await adapter.applyAskConfiguration({
      report: {
        matrix: {
          search_mode_visible: true,
          research_mode_visible: false,
          model_picker_visible: false,
          thinking_effort_visible: false,
        },
        aliases: {
          modes: { search: ["search"] },
          model_classes: {},
          efforts: {},
        },
      },
      request: { mode: "search" },
    });
    expect(search.getAttribute("aria-checked")).toBe("true");
    expect(searchChip.hasAttribute("hidden")).toBe(false);

    expect(adapter.readAskSnapshot()).toMatchObject({
      hasAssistantMessage: true,
      text: "Final answer.",
      state: "done",
      conversationId: "conv-command",
      messageId: "msg-command",
      currentUrl: "https://chatgpt.com/c/conv-command",
    });
    dom.window.close();
  });

  it("resolves renamed semantic controls without ChatGPT test ids or composer pill classes", async () => {
    const runtimeSource = loadPageAdapterSource();
    const dom = new JSDOM(
      `<!doctype html><html><body data-stream-state="done" data-selected-effort="standard">
        <main>
          <form>
            <div role="textbox" aria-label="Ask anything" contenteditable="true"></div>
            <button
              id="engine-trigger"
              type="button"
              aria-label="Choose engine"
              aria-haspopup="listbox"
              aria-controls="engine-list"
              aria-expanded="false"
            >Engine</button>
            <button
              id="depth-trigger"
              type="button"
              aria-label="Reasoning depth"
              aria-haspopup="listbox"
              aria-controls="reasoning-depth-list"
              aria-expanded="false"
            >Depth</button>
            <button
              id="tools-trigger"
              type="button"
              aria-label="Composer tools"
              aria-haspopup="menu"
              aria-controls="tools-menu"
              aria-expanded="false"
            >Tools</button>
            <button id="semantic-submit" type="button" aria-label="Launch message">Launch</button>
          </form>
          <div id="engine-list" role="listbox" hidden>
            <div id="engine-fast" role="option" aria-selected="true" hidden>Quick answer</div>
            <div id="engine-thinking" role="option" aria-selected="false" hidden>Deep reasoning</div>
          </div>
          <div id="reasoning-depth-list" role="listbox" hidden>
            <div id="depth-standard" role="option" aria-selected="true" hidden>Balanced</div>
            <div id="depth-extended" role="option" aria-selected="false" hidden>Deep analysis</div>
          </div>
          <div id="tools-menu" role="menu" hidden>
            <div id="browse-online" role="option" aria-selected="false" hidden>Browse online</div>
          </div>
        </main>
      </body></html>`,
      {
        url: "https://chatgpt.com/c/conv-semantic-runtime",
        runScripts: "outside-only",
      },
    );
    const { window } = dom;
    const adapter = window.eval(
      `${runtimeSource.replace(
        /^export\s+/gm,
        "",
      )}\ncreateTablinqPageAdapterRuntime();`,
    );
    const showControlled = (triggerId: string) => {
      const trigger = window.document.querySelector(`#${triggerId}`);
      if (!(trigger instanceof window.HTMLElement)) {
        throw new Error(`missing ${triggerId}`);
      }
      const root = window.document.getElementById(
        trigger.getAttribute("aria-controls") ?? "",
      );
      if (!(root instanceof window.HTMLElement)) {
        throw new Error(`missing controlled root for ${triggerId}`);
      }
      trigger.addEventListener("click", () => {
        const expanded = trigger.getAttribute("aria-expanded") === "true";
        trigger.setAttribute("aria-expanded", expanded ? "false" : "true");
        root.toggleAttribute("hidden", expanded);
        root
          .querySelectorAll("[role='option']")
          .forEach((element) => element.toggleAttribute("hidden", expanded));
      });
    };
    showControlled("engine-trigger");
    showControlled("depth-trigger");
    showControlled("tools-trigger");

    window.document
      .querySelector("#engine-thinking")
      ?.addEventListener("click", () => {
        window.document.body.dataset.selectedModel = "thinking";
        window.document
          .querySelector("#engine-trigger")
          ?.setAttribute("aria-expanded", "false");
        window.document
          .querySelector("#engine-list")
          ?.setAttribute("hidden", "");
        window.document
          .querySelectorAll("#engine-list [role='option']")
          .forEach((element) => {
            element.setAttribute(
              "aria-selected",
              element.id === "engine-thinking" ? "true" : "false",
            );
            element.setAttribute("hidden", "");
          });
      });
    window.document
      .querySelector("#depth-standard")
      ?.addEventListener("click", () => {
        window.document.body.dataset.selectedEffort = "standard";
      });
    window.document
      .querySelector("#browse-online")
      ?.addEventListener("click", () => {
        window.document.body.dataset.selectedMode = "search";
        window.document
          .querySelector("#tools-trigger")
          ?.setAttribute("aria-expanded", "false");
        window.document
          .querySelector("#tools-menu")
          ?.setAttribute("hidden", "");
        window.document
          .querySelector("#browse-online")
          ?.setAttribute("aria-selected", "true");
        window.document
          .querySelector("#browse-online")
          ?.setAttribute("hidden", "");
      });
    window.document
      .querySelector("#semantic-submit")
      ?.addEventListener("click", () => {
        window.document.body.dataset.submitted = "true";
      });

    expect(adapter.composerReady()).toBe(true);
    expect(adapter.readSubmitState()).toMatchObject({
      submitVisible: true,
      submitEnabled: true,
      keyboardReady: false,
    });

    adapter.fillComposer({ prompt: "Semantic runtime prompt" });
    await adapter.applyAskConfiguration({
      report: {
        matrix: {
          search_mode_visible: true,
          research_mode_visible: false,
          model_picker_visible: true,
          thinking_effort_visible: true,
        },
        aliases: {
          modes: { search: ["browse online"] },
          model_classes: {
            instant: ["quick answer"],
            thinking: ["deep reasoning"],
          },
          efforts: {
            standard: ["balanced"],
            extended: ["deep analysis"],
          },
        },
      },
      request: {
        mode: "search",
        modelClass: "thinking",
        effort: "standard",
      },
    });
    expect(window.document.body.dataset.selectedMode).toBe("search");
    expect(window.document.body.dataset.selectedModel).toBe("thinking");
    expect(window.document.body.dataset.selectedEffort).toBe("standard");

    expect(adapter.clickSubmit()).toBe(true);
    expect(window.document.body.dataset.submitted).toBe("true");
    dom.window.close();
  });

  it("keeps selectorless runtime actions anchored to the active composer", async () => {
    const runtimeSource = loadPageAdapterSource();
    const dom = new JSDOM(
      `<!doctype html><html><body data-stream-state="done" data-selected-model="thinking" data-selected-effort="extended">
        <main>
          <section id="selectorless-decoys">
            <button type="button" aria-label="Search chats">Search chats</button>
            <button type="button" id="stale-send" aria-label="Send prompt">Send</button>
          </section>
          <form>
            <div role="textbox" aria-label="Message ChatGPT" contenteditable="true"></div>
            <button
              id="engine-trigger"
              type="button"
              aria-label="Choose engine"
              aria-haspopup="listbox"
              aria-controls="engine-list"
              aria-expanded="false"
            >Engine</button>
            <button
              id="depth-trigger"
              type="button"
              aria-label="Reasoning depth"
              aria-haspopup="listbox"
              aria-controls="depth-list"
              aria-expanded="false"
            >Depth</button>
            <button
              id="tools-trigger"
              type="button"
              aria-label="Add files and more"
              aria-haspopup="listbox"
              aria-controls="tools-list"
              aria-expanded="false"
            >Tools</button>
            <button id="active-send" type="button" aria-label="Send message">Send</button>
          </form>
          <aside>
            <div id="engine-list" role="listbox" hidden>
              <div id="engine-instant" role="option" data-model="instant" aria-selected="false" hidden>Instant</div>
              <div id="engine-thinking" role="option" data-model="thinking" aria-selected="true" hidden>Thinking</div>
            </div>
            <div id="depth-list" role="listbox" hidden>
              <div id="depth-standard" role="option" data-effort="standard" aria-selected="false" hidden>Standard</div>
              <div id="depth-extended" role="option" data-effort="extended" aria-selected="true" hidden>Extended</div>
            </div>
            <div id="tools-list" role="listbox" hidden>
              <div id="browse-web" role="option" data-mode="search" aria-selected="false" hidden>Browse the web</div>
            </div>
          </aside>
        </main>
      </body></html>`,
      {
        url: "https://chatgpt.com/c/conv-selectorless-runtime",
        runScripts: "outside-only",
      },
    );
    const { window } = dom;
    const adapter = window.eval(
      `${runtimeSource.replace(
        /^export\s+/gm,
        "",
      )}\ncreateTablinqPageAdapterRuntime();`,
    );
    const openControlled = (triggerId: string) => {
      const trigger = window.document.getElementById(triggerId);
      if (!(trigger instanceof window.HTMLElement)) {
        throw new Error(`missing ${triggerId}`);
      }
      const root = window.document.getElementById(
        trigger.getAttribute("aria-controls") ?? "",
      );
      if (!(root instanceof window.HTMLElement)) {
        throw new Error(`missing popup for ${triggerId}`);
      }
      trigger.addEventListener("click", () => {
        trigger.setAttribute("aria-expanded", "true");
        root.removeAttribute("hidden");
        root
          .querySelectorAll("[role='option']")
          .forEach((element) => element.removeAttribute("hidden"));
      });
    };
    openControlled("engine-trigger");
    openControlled("depth-trigger");
    openControlled("tools-trigger");

    window.document
      .querySelector("#engine-instant")
      ?.addEventListener("click", () => {
        window.document.body.dataset.selectedModel = "instant";
      });
    window.document
      .querySelector("#depth-standard")
      ?.addEventListener("click", () => {
        window.document.body.dataset.selectedEffort = "standard";
      });
    window.document
      .querySelector("#browse-web")
      ?.addEventListener("click", () => {
        window.document.body.dataset.selectedMode = "search";
      });
    window.document
      .querySelector("#stale-send")
      ?.addEventListener("click", () => {
        window.document.body.dataset.staleSendClicks = String(
          Number.parseInt(
            window.document.body.dataset.staleSendClicks ?? "0",
            10,
          ) + 1,
        );
      });
    window.document
      .querySelector("#active-send")
      ?.addEventListener("click", () => {
        window.document.body.dataset.submitted = "true";
      });

    expect(adapter.composerReady()).toBe(true);
    expect(adapter.readSubmitState()).toMatchObject({
      submitVisible: true,
      submitEnabled: true,
      keyboardReady: false,
    });

    adapter.fillComposer({ prompt: "Selectorless runtime prompt" });
    expect(
      window.document.querySelector("[role='textbox']")?.textContent?.trim(),
    ).toBe("Selectorless runtime prompt");

    await adapter.applyAskConfiguration({
      report: {
        matrix: {
          search_mode_visible: true,
          research_mode_visible: false,
          model_picker_visible: true,
          thinking_effort_visible: true,
        },
        aliases: {
          modes: { search: ["browse the web"] },
          model_classes: {
            instant: ["instant"],
            thinking: ["thinking"],
          },
          efforts: {
            standard: ["standard"],
            extended: ["extended"],
          },
        },
      },
      request: {
        mode: "search",
        modelClass: "instant",
      },
    });
    expect(window.document.body.dataset.selectedMode).toBe("search");
    expect(window.document.body.dataset.selectedModel).toBe("instant");

    expect(adapter.clickSubmit()).toBe(true);
    expect(window.document.body.dataset.submitted).toBe("true");
    expect(window.document.body.dataset.staleSendClicks ?? "0").toBe("0");
    dom.window.close();
  });

  it("serializes rich assistant markdown into terminal-safe markdown text", async () => {
    const runtimeSource = loadPageAdapterSource();
    const expected = [
      "# Heading One",
      "## Heading Two",
      "",
      "Plain text line.",
      "",
      "1. First numbered item",
      "2. Second numbered item",
      "",
      "- Bullet one",
      "- Bullet two",
      "",
      "> Blockquote line",
      "",
      "Inline `code` example.",
      "",
      "```python",
      "def hello():",
      '    return "world"',
      "```",
      "",
      "**bold**",
      "*italic*",
      "***bold italic***",
      "",
      "| col1 | col2 |",
      "| --- | --- |",
      "| A | B |",
      "",
      "Final sentence.",
    ].join("\n");

    const dom = new JSDOM("<!doctype html><html><body></body></html>", {
      url: "http://127.0.0.1:3847/?variant=live",
      runScripts: "outside-only",
    });
    const { window } = dom;
    const article = window.document.createElement("article");
    article.setAttribute("data-slot", "assistant-message");
    article.hidden = false;
    article.setAttribute("data-message-author-role", "assistant");
    article.setAttribute("data-message-id", "msg-rich-markdown");
    article.innerHTML = `
          <div class="markdown prose dark:prose-invert w-full wrap-break-word light markdown-new-styling">
            <h1>Heading One</h1>
            <h2>Heading Two</h2>
            <p>Plain text line.</p>
            <ol>
              <li><p>First numbered item</p></li>
              <li><p>Second numbered item</p></li>
            </ol>
            <ul>
              <li><p>Bullet one</p></li>
              <li><p>Bullet two</p></li>
            </ul>
            <blockquote><p>Blockquote line</p></blockquote>
            <p>Inline <code>code</code> example.</p>
            <pre>
              <div class="text-sm font-medium">Python</div>
              <button aria-label="Copy"></button>
              <button aria-label="Run code">Run</button>
              <div class="cm-content">def hello():\n    return "world"</div>
            </pre>
            <p><strong>bold</strong><br><em>italic</em><br><em><strong>bold italic</strong></em></p>
            <div class="table-container">
              <table>
                <thead>
                  <tr><th>col1</th><th>col2</th></tr>
                </thead>
                <tbody>
                  <tr><td>A</td><td>B</td></tr>
                </tbody>
              </table>
              <button aria-label="Copy table"></button>
            </div>
            <p>Final sentence.</p>
          </div>
        `;
    window.document.body.append(article);
    window.document.body.dataset.conversationId = "conv-rich-markdown";
    window.document.body.dataset.lastMessageId = "msg-rich-markdown";
    window.document.body.dataset.streamState = "done";

    const snapshot = window.eval(
      `${runtimeSource.replace(
        /^export\s+/gm,
        "",
      )}\ncreateTablinqPageAdapterRuntime().readAskSnapshot();`,
    );

    expect(snapshot.text).toBe(expected);
    dom.window.close();
  });

  it("serializes inline KaTeX annotations as Markdown math", () => {
    const tex = String.raw`z_k=e^{2\pi i k/5}`;
    const text = readAssistantMarkdownSnapshotText(`
      <p>
        The roots satisfy
        ${katexHtml(
          tex,
          "<span>z</span><span>k</span><span>=</span><span>e</span><span>2</span><span>π</span><span>i</span><span>k</span><span>/</span><span>5</span>",
        )}.
      </p>
    `);

    expect(text).toBe(`The roots satisfy $${tex}$.`);
  });

  it("serializes display KaTeX annotations as Markdown math blocks", () => {
    const tex = String.raw`\oint_\gamma f(z)\,dz = 2\pi i \sum \operatorname{Res}(f,a_k)`;
    const text = readAssistantMarkdownSnapshotText(`
      <div class="katex-display">
        ${katexHtml(tex)}
      </div>
    `);

    expect(text).toBe(["$$", tex, "$$"].join("\n"));
  });

  it("uses TeX annotations instead of rendered KaTeX token spans", () => {
    const tex = String.raw`\frac{x^2+y^2}{z_1}`;
    const text = readAssistantMarkdownSnapshotText(`
      <p>${katexHtml(
        tex,
        "<span>VISIBLE</span><span>TOKENIZED</span><span>FRACTION</span>",
      )}</p>
    `);

    expect(text).toBe(`$${tex}$`);
    expect(text).not.toContain("VISIBLE");
    expect(text).not.toContain("TOKENIZED");
  });

  it("keeps complex streaming math expressions as a single LaTeX expression", () => {
    const tex = String.raw`\int_{|z|=2} \frac{e^z}{z(z-1)}\,dz = 2\pi i(e-1)`;
    const text = readAssistantMarkdownSnapshotText(
      `<p>The contour integral is ${katexHtml(tex)}.</p>`,
      "running",
    );

    expect(text).toBe(`The contour integral is $${tex}$.`);
    expect(text.split("\n")).toHaveLength(1);
  });

  it("serializes complex markdown table cells without breaking table columns", async () => {
    const runtimeSource = loadPageAdapterSource();
    const expected = [
      "| Key | Value | Notes |",
      "| --- | --- | --- |",
      "| Simple | Alpha | Beta |",
      "| Pipe | A \\| B | stays one cell |",
      "| Code | `left \\| right` | inline code stays inline |",
      "| Link | [Docs \\| API](https://example.test/docs\\|api) | link stays parseable |",
      "| Multiline | first<br>second | still one cell |",
      "| Ragged | fewer cells |",
    ].join("\n");

    const dom = new JSDOM("<!doctype html><html><body></body></html>", {
      url: "http://127.0.0.1:3847/?variant=live",
      runScripts: "outside-only",
    });
    const { window } = dom;
    const article = window.document.createElement("article");
    article.setAttribute("data-slot", "assistant-message");
    article.hidden = false;
    article.setAttribute("data-message-author-role", "assistant");
    article.setAttribute("data-message-id", "msg-complex-table");
    article.innerHTML = `
      <div class="markdown">
        <table>
          <thead>
            <tr><th>Key</th><th>Value</th><th>Notes</th></tr>
          </thead>
          <tbody>
            <tr><td>Simple</td><td>Alpha</td><td>Beta</td></tr>
            <tr><td>Pipe</td><td>A | B</td><td>stays one cell</td></tr>
            <tr><td>Code</td><td><code>left | right</code></td><td>inline code stays inline</td></tr>
            <tr><td>Link</td><td><a href="https://example.test/docs|api">Docs | API</a></td><td>link stays parseable</td></tr>
            <tr><td>Multiline</td><td>first<br>second</td><td>still one cell</td></tr>
            <tr><td>Ragged</td><td>fewer cells</td></tr>
          </tbody>
        </table>
      </div>
    `;
    window.document.body.append(article);
    window.document.body.dataset.conversationId = "conv-complex-table";
    window.document.body.dataset.lastMessageId = "msg-complex-table";
    window.document.body.dataset.streamState = "done";

    const snapshot = window.eval(
      `${runtimeSource.replace(
        /^export\s+/gm,
        "",
      )}\ncreateTablinqPageAdapterRuntime().readAskSnapshot();`,
    );

    expect(snapshot.text).toBe(expected);
    dom.window.close();
  });

  it("serializes mermaid preview blocks into explicit text instead of a bare mermaid label", async () => {
    const runtimeSource = loadPageAdapterSource();
    const dom = new JSDOM("<!doctype html><html><body></body></html>", {
      url: "https://chatgpt.com/c/conv-mermaid-preview",
      runScripts: "outside-only",
    });
    const { window } = dom;
    const article = window.document.createElement("article");
    article.setAttribute("data-slot", "assistant-message");
    article.hidden = false;
    article.setAttribute("data-message-author-role", "assistant");
    article.setAttribute("data-message-id", "msg-mermaid-preview");
    article.innerHTML = `
      <div class="markdown">
        <pre class="overflow-visible! px-0!">
          <div class="sticky z-2 select-none top-(--sticky-padding-top)">
            <div class="flex w-full items-center justify-between py-1.5 ps-4 pe-1.5 font-sans md:ps-5">
              <div>Mermaid</div>
              <div class="flex flex-row items-center gap-0.5 justify-self-end">
                <button aria-label="Download PNG" type="button"></button>
                <button aria-label="Code" type="button" aria-pressed="false"></button>
                <button aria-label="Preview" type="button" aria-pressed="true"></button>
              </div>
            </div>
          </div>
          <div class="flex w-full flex-col items-center">
            <img src="data:image/svg+xml;utf8,${encodeURIComponent(
              '<svg xmlns="http://www.w3.org/2000/svg"><text>Exercise</text><text>Recovery Loop</text></svg>',
            )}" />
          </div>
        </pre>
      </div>
    `;
    window.document.body.append(article);
    window.document.body.dataset.conversationId = "conv-mermaid-preview";
    window.document.body.dataset.lastMessageId = "msg-mermaid-preview";
    window.document.body.dataset.streamState = "done";

    const snapshot = window.eval(
      `${runtimeSource.replace(
        /^export\s+/gm,
        "",
      )}\ncreateTablinqPageAdapterRuntime().readAskSnapshot();`,
    );

    expect(snapshot.text).toBe(
      "Mermaid diagram preview: Exercise; Recovery Loop.",
    );
    dom.window.close();
  });

  it("ignores transient hidden inline nodes so ask streaming can keep a stable prefix", async () => {
    const runtimeSource = loadPageAdapterSource();
    const dom = new JSDOM("<!doctype html><html><body></body></html>", {
      url: "http://127.0.0.1:3847/?variant=live",
      runScripts: "outside-only",
    });
    const { window } = dom;
    const article = window.document.createElement("article");
    article.setAttribute("data-slot", "assistant-message");
    article.hidden = false;
    article.setAttribute("data-message-author-role", "assistant");
    article.setAttribute("data-message-id", "msg-streaming-rich");
    article.innerHTML = `
      <div class="markdown prose dark:prose-invert w-full wrap-break-word light markdown-new-styling">
        <h1>🧩 Backe<span hidden> </span>nd Programming Languages — Deep Comparison (2025)</h1>
        <p>Below is a </p>
      </div>
    `;
    window.document.body.append(article);
    window.document.body.dataset.conversationId = "conv-streaming-rich";
    window.document.body.dataset.lastMessageId = "msg-streaming-rich";
    window.document.body.dataset.streamState = "running";

    const readAskSnapshot = () =>
      window.eval(
        `${runtimeSource.replace(
          /^export\s+/gm,
          "",
        )}\ncreateTablinqPageAdapterRuntime().readAskSnapshot();`,
      );

    const firstSnapshot = readAskSnapshot();
    expect(firstSnapshot.text).toBe(
      "# 🧩 Backend Programming Languages — Deep Comparison (2025)\n\nBelow is a",
    );

    article.innerHTML = `
      <div class="markdown prose dark:prose-invert w-full wrap-break-word light markdown-new-styling">
        <h1>🧩 Backend Programming Languages — Deep Comparison (2025)</h1>
        <p>Below is a <strong>structured, nested comparison</strong> of the most widely used backend languages.</p>
      </div>
    `;

    const secondSnapshot = readAskSnapshot();
    const chunks: string[] = [];
    const state = {
      emittedText: "",
      nextSeq: 0,
      knownMessageId: null,
      pendingMessageId: null,
      pendingMessageText: "",
      lastSnapshotSignature: null,
    };
    const command = {
      jobId: "job-streaming-rich",
    } as Parameters<typeof applyAskSnapshot>[0];

    const firstProgressed = await applyAskSnapshot(
      command,
      state,
      firstSnapshot,
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );
    const secondProgressed = await applyAskSnapshot(
      command,
      state,
      secondSnapshot,
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );

    expect(firstProgressed).toBe(true);
    expect(secondProgressed).toBe(true);
    expect(chunks).toEqual([]);
    dom.window.close();
  });

  it("reads modern section-based assistant turns and tracks stable turn progress", async () => {
    const runtimeSource = loadPageAdapterSource();
    const dom = new JSDOM(
      "<!doctype html><html><body><main></main></body></html>",
      {
        url: "https://chatgpt.com/c/conv-section-stream",
        runScripts: "outside-only",
      },
    );
    const { window } = dom;
    const main = window.document.querySelector("main");
    if (!(main instanceof window.HTMLElement)) {
      throw new Error("expected main element");
    }

    main.innerHTML = `
      <section data-testid="conversation-turn-1">
        <h4 class="sr-only">You said:</h4>
        <div class="user-message-bubble-color">
          <div>compare martial arts</div>
        </div>
      </section>
      <section data-testid="conversation-turn-2">
        <h4 class="sr-only">ChatGPT said:</h4>
        <div class="markdown">
          <p>Earlier answer.</p>
        </div>
      </section>
    `;
    window.document.body.dataset.streamState = "idle";

    const adapter = window.eval(
      `${runtimeSource.replace(/^export\s+/gm, "")}\ncreateTablinqPageAdapterRuntime();`,
    );
    const baseline = adapter.readAskBaseline();
    expect(baseline.assistantMessageCount).toBe(1);
    expect(baseline.messageId).toBe("conversation-turn-2");
    expect(baseline.text).toBe("Earlier answer.");

    main.insertAdjacentHTML(
      "beforeend",
      `
        <section data-testid="conversation-turn-3">
          <h4 class="sr-only">You said:</h4>
          <div class="user-message-bubble-color">
            <div>give me a shorter comparison</div>
          </div>
        </section>
        <section data-testid="conversation-turn-4">
          <h4 class="sr-only">ChatGPT said:</h4>
          <div class="markdown">
            <p>High-level</p>
          </div>
        </section>
      `,
    );
    window.document.body.dataset.streamState = "running";

    const firstSnapshot = adapter.readAskSnapshot({ baseline });
    expect(firstSnapshot.messageId).toBe("conversation-turn-4");
    expect(firstSnapshot.text).toBe("High-level");
    expect(firstSnapshot.state).toBe("running");

    const chunks: string[] = [];
    const state = {
      emittedText: "",
      nextSeq: 0,
      knownMessageId: null,
      pendingMessageId: null,
      pendingMessageText: "",
      lastSnapshotSignature: null,
    };
    const command = {
      jobId: "job-section-turn-streaming",
    } as Parameters<typeof applyAskSnapshot>[0];

    const firstProgressed = await applyAskSnapshot(
      command,
      state,
      firstSnapshot,
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );
    expect(firstProgressed).toBe(true);

    const latestTurn = main.querySelector(
      "[data-testid='conversation-turn-4'] .markdown",
    );
    if (!(latestTurn instanceof window.HTMLElement)) {
      throw new Error("expected latest assistant markdown");
    }
    latestTurn.innerHTML = "<p>High-level comparison</p>";

    const secondSnapshot = adapter.readAskSnapshot({ baseline });
    const secondProgressed = await applyAskSnapshot(
      command,
      state,
      secondSnapshot,
      (chunk) => {
        if (chunk.kind === "delta") {
          chunks.push(chunk.text);
        }
      },
    );

    expect(secondSnapshot.messageId).toBe("conversation-turn-4");
    expect(secondSnapshot.text).toBe("High-level comparison");
    expect(secondProgressed).toBe(true);
    expect(chunks).toEqual([]);
    dom.window.close();
  });

  it("prefers modern assistant sections over nested created markers for inline text documents", () => {
    const runtimeSource = loadPageAdapterSource();
    const dom = new JSDOM(
      "<!doctype html><html><body><main></main></body></html>",
      {
        url: "https://chatgpt.com/c/conv-textdoc",
        runScripts: "outside-only",
      },
    );
    const { window } = dom;
    const main = window.document.querySelector("main");
    if (!(main instanceof window.HTMLElement)) {
      throw new Error("expected main element");
    }

    main.innerHTML = `
      <section data-testid="conversation-turn-1" data-turn="user">
        <h4 class="sr-only">You said:</h4>
        <div class="user-message-bubble-color">Return the file.</div>
      </section>
      <section data-testid="conversation-turn-2" data-turn="assistant">
        <h4 class="sr-only">ChatGPT said:</h4>
        <div id="textdoc-message-1" class="popover">
          <div>
            <span>Tablinq Smoke Report Template</span>
            <button aria-label="Copy">Copy</button>
            <button>Download</button>
          </div>
          <section class="popover">
            <div class="markdown ProseMirror" contenteditable="false">
              <h1><span>Tablinq Smoke Report</span></h1>
              <h2><span>Recovery</span></h2>
              <ul>
                <li><p><span>Final State:</span></p></li>
              </ul>
            </div>
          </section>
        </div>
        <div data-message-author-role="assistant">
          <p>Created.</p>
        </div>
        <div data-message-author-role="assistant"></div>
      </section>
    `;
    window.document.body.dataset.streamState = "done";

    const snapshot = window.eval(
      `${runtimeSource.replace(
        /^export\s+/gm,
        "",
      )}\ncreateTablinqPageAdapterRuntime().readAskSnapshot();`,
    );

    expect(snapshot.messageId).toBe("conversation-turn-2");
    expect(snapshot.text).toBe(
      "# Tablinq Smoke Report\n## Recovery\n\n- Final State:",
    );
    dom.window.close();
  });

  it("tracks provisional ask text without emitting chunks until finalization", async () => {
    const command = {
      jobId: "job-provisional-ask-message-id",
    } as Parameters<typeof applyAskSnapshot>[0];
    const state = {
      emittedText: "",
      nextSeq: 0,
      knownMessageId: null,
      pendingMessageId: null,
      pendingMessageText: "",
      lastSnapshotSignature: null,
    };
    const chunks: string[] = [];
    const onChunk = (chunk: { kind: string; text: string }) => {
      if (chunk.kind === "delta") {
        chunks.push(chunk.text);
      }
    };
    const provisionalSnapshot = {
      hasAssistantMessage: true,
      text: "## 🧩 M ovement Modalities Comparison",
      state: "running",
      failureCode: null,
      failureMessage: null,
      retryVisible: false,
      conversationId: "conv-modalities",
      messageId: null,
      thoughtDividerBeforeMessage: false,
      currentUrl: "https://chatgpt.com/c/conv-modalities",
    };
    const identifiedSnapshot = {
      ...provisionalSnapshot,
      text: [
        "## 🧩 Movement Modalities Comparison",
        "",
        "```",
        "Physical Activity",
        "├── 🧘 Stretching",
        "└── 🏋️ Gym Workout",
        "```",
      ].join("\n"),
      messageId: "msg-modalities",
    };
    const advancedSnapshot = {
      ...identifiedSnapshot,
      text: [
        identifiedSnapshot.text,
        "",
        "## 📊 Side-by-Side Attribute Matrix",
        "",
        "| Attribute | Stretching | Gym Workout |",
      ].join("\n"),
    };

    const provisionalProgressed = await applyAskSnapshot(
      command,
      state,
      provisionalSnapshot,
      onChunk,
    );
    const identifiedProgressed = await applyAskSnapshot(
      command,
      state,
      identifiedSnapshot,
      onChunk,
    );
    const advancedProgressed = await applyAskSnapshot(
      command,
      state,
      advancedSnapshot,
      onChunk,
    );

    expect(provisionalProgressed).toBe(true);
    expect(identifiedProgressed).toBe(true);
    expect(advancedProgressed).toBe(true);
    expect(chunks).toEqual([]);
  });

  it("tracks a completed provisional ask snapshot when no message id ever arrives", async () => {
    const command = {
      jobId: "job-provisional-ask-done",
    } as Parameters<typeof applyAskSnapshot>[0];
    const state = {
      emittedText: "",
      nextSeq: 0,
      knownMessageId: null,
      pendingMessageId: null,
      pendingMessageText: "",
      lastSnapshotSignature: null,
    };
    const chunks: string[] = [];
    const onChunk = (chunk: { kind: string; text: string }) => {
      if (chunk.kind === "delta") {
        chunks.push(chunk.text);
      }
    };
    const runningSnapshot = {
      hasAssistantMessage: true,
      text: "Reply",
      state: "running",
      failureCode: null,
      failureMessage: null,
      retryVisible: false,
      conversationId: "conv-provisional-done",
      messageId: null,
      thoughtDividerBeforeMessage: false,
      currentUrl: "https://chatgpt.com/c/conv-provisional-done",
    };
    const doneSnapshot = {
      ...runningSnapshot,
      text: "Reply exactly with ASK_OK",
      state: "done",
    };

    const runningProgressed = await applyAskSnapshot(
      command,
      state,
      runningSnapshot,
      onChunk,
    );
    const doneProgressed = await applyAskSnapshot(
      command,
      state,
      doneSnapshot,
      onChunk,
    );

    expect(runningProgressed).toBe(true);
    expect(doneProgressed).toBe(true);
    expect(chunks).toEqual([]);
    expect(state.emittedText).toBe("");
    expect(state.pendingMessageText).toBe(doneSnapshot.text);
    expect(state.knownMessageId).toBeNull();
  });
});
