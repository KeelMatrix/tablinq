// @vitest-environment jsdom

import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import addFormats from "ajv-formats";
import Ajv2020 from "ajv/dist/2020";
import { loadSchema } from "../../browser-adapter/src/protocol";
import { probeDocument } from "../src/probe";
import {
  accessibleProbeName,
  isVisibleProbeElement,
  orderCapabilityStrategies,
  probeElementPath,
  probeTextHasToken,
  probeDocument as runtimeProbeDocument,
} from "../src/probe.runtime.js";
import {
  createBridgeController,
  type BridgeStatus,
} from "../src/service-worker";

const ajv = new Ajv2020({ allErrors: true, strict: false });
addFormats(ajv);
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");

const validateCapabilityReport = ajv.compile(
  loadSchema<Record<string, unknown>>("capability-report"),
);

describe("extension capability probe", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    document.body.dataset.uiVariant = "classic";
  });

  it("exports probeDocument from the canonical runtime source", () => {
    expect(typeof runtimeProbeDocument).toBe("function");
    expect(runtimeProbeDocument).toBe(probeDocument);
  });

  it("keeps DOM helper behavior focused and directly testable", () => {
    document.body.innerHTML = `
      <section id="root">
        <label id="prompt-label" for="prompt"> Message ChatGPT </label>
        <textarea id="prompt" aria-labelledby="prompt-label"></textarea>
        <button id="hidden" hidden>Send</button>
      </section>
    `;

    const prompt = document.querySelector("#prompt")!;
    const hidden = document.querySelector("#hidden")!;

    expect(accessibleProbeName(prompt)).toBe("message chatgpt");
    expect(isVisibleProbeElement(prompt)).toBe(true);
    expect(isVisibleProbeElement(hidden)).toBe(false);
    expect(probeElementPath(prompt)).toContain("textarea#prompt");
    expect(probeTextHasToken("browse the web", ["browse the web"])).toBe(true);
  });

  it("orders capability strategies from selector pack and preferred strategy metadata", () => {
    const strategies = [
      { id: "fallback", score: 0.5 },
      { id: "preferred", score: 0.75 },
      { id: "pack-first", score: 0.1 },
    ];

    const ordered = orderCapabilityStrategies({
      capability: "composer_ready",
      strategies,
      preferredStrategies: { composer_ready: "preferred" },
      selectorPack: {
        capabilities: {
          composer_ready: [{ strategy_id: "pack-first", score: 1 }],
        },
      },
    });

    expect(ordered.map((strategy) => strategy.id)).toEqual([
      "preferred",
      "pack-first",
      "fallback",
    ]);
  });

  it("keeps the content probe generated from the runtime source", () => {
    execFileSync(
      process.execPath,
      ["packages/extension/scripts/generate-probe-content.mjs", "--check"],
      {
        cwd: repoRoot,
        stdio: "pipe",
      },
    );
  });

  it("detects the classic fake harness controls", () => {
    document.body.innerHTML = `
      <section>
        <div>
          <div data-slot="model-picker" role="group" aria-label="Model picker">
            <button type="button" data-model="instant" aria-pressed="false">GPT-5.3 Instant</button>
            <button type="button" data-model="thinking" aria-pressed="true">GPT-5.4 Thinking</button>
          </div>
          <button type="button" data-mode="research">Research</button>
          <div data-slot="effort-toggle" role="group" aria-label="Thinking effort">
            <button type="button" data-effort="standard" aria-pressed="true">Standard</button>
            <button type="button" data-effort="extended" aria-pressed="false">Extended</button>
          </div>
          <button type="button" data-slot="upload-entry">Attach</button>
        </div>
        <div>
          <textarea data-slot="composer" aria-label="Prompt composer"></textarea>
          <button type="button" data-action="submit">Send</button>
        </div>
        <article>
          <a href="/artifact.txt" download data-slot="artifact-link">artifact.txt</a>
        </article>
      </section>
    `;

    const message = probeDocument({
      reportedAt: "2026-04-12T10:00:00Z",
    });

    expect(message.report.matrix).toEqual({
      composer_ready: true,
      submit_available: true,
      model_picker_visible: true,
      thinking_effort_visible: true,
      search_mode_visible: false,
      research_mode_visible: true,
      upload_control_visible: true,
      downloadable_artifacts_observable: true,
    });
    expect(message.runtime.strategy_matches.composer_ready).toBe(
      "data-slot-composer",
    );
    expect(message.runtime.strategy_matches.model_picker_visible).toBe(
      "data-slot-model-picker",
    );
    expect(message.report.aliases.model_classes.instant).toContain("instant");
    expect(message.report.aliases.model_classes.thinking).toContain("thinking");
    expect(message.report.aliases.efforts.extended).toContain("extended");
    expect(message.report.aliases.modes.research).toContain("deep research");
    expect(message.report.aliases.modes.search).toBeUndefined();
    expect(message.report.observed_ui.selected_model).toMatchObject({
      status: "observed",
      picker_visible: true,
      selected_model_class: "thinking",
    });
    expect(message.report.observed_ui.thinking_effort).toMatchObject({
      status: "observed",
      controls_visible: true,
      selected_effort: "standard",
    });
    expect(message.report.observed_ui.search_mode).toMatchObject({
      status: "not_observed",
      selected: false,
    });
    expect(message.report.observed_ui.research_flow).toMatchObject({
      status: "observed",
      entrypoint_visible: true,
      on_research_url: false,
    });
    expect(message.runtime.dom_neighborhoods.length).toBeGreaterThan(3);
    expect(validateCapabilityReport(message.report)).toBe(true);
    expect(validateCapabilityReport.errors).toBeNull();
  });

  it("falls back to aria and neighborhood strategies on the alternate UI", () => {
    document.body.dataset.uiVariant = "aria";
    document.body.innerHTML = `
      <section aria-label="assistant workspace">
        <div>
          <div role="group" aria-label="Choose engine">
            <button type="button" data-model="instant" aria-pressed="false">
              Instant answers
            </button>
            <button type="button" data-model="thinking" aria-pressed="true">
              Deep thinking
            </button>
          </div>
          <button type="button" aria-pressed="false">Deep research</button>
          <div role="group" aria-label="Reasoning effort">
            <button type="button" data-effort="standard" aria-pressed="true">
              Standard reasoning
            </button>
            <button type="button" data-effort="extended" aria-pressed="false">
              Extended reasoning
            </button>
          </div>
          <label for="upload-input">Add files</label>
          <input id="upload-input" type="file" hidden />
        </div>
        <div>
          <div role="textbox" aria-label="Message ChatGPT" contenteditable="true"></div>
          <button type="button" aria-label="Submit prompt">Go</button>
        </div>
        <article>
          <a href="/artifact.txt" download>Download artifact.txt</a>
        </article>
      </section>
    `;

    const message = probeDocument({
      reportedAt: "2026-04-12T10:00:00Z",
    });

    expect(message.report.matrix).toEqual({
      composer_ready: true,
      submit_available: true,
      model_picker_visible: true,
      thinking_effort_visible: true,
      search_mode_visible: false,
      research_mode_visible: true,
      upload_control_visible: true,
      downloadable_artifacts_observable: true,
    });
    expect(message.runtime.strategy_matches.composer_ready).toBe(
      "aria-textbox",
    );
    expect(message.runtime.strategy_matches.model_picker_visible).toBe(
      "inline-model-options",
    );
    expect(
      message.runtime.strategy_matches.search_mode_visible,
    ).toBeUndefined();
    expect(message.report.aliases.modes.search).toBeUndefined();
    expect(message.report.aliases.modes.research).toContain("deep research");
    expect(message.report.aliases.model_classes.instant).toContain("instant");
    expect(message.report.aliases.model_classes.thinking).toContain("thinking");
    expect(message.report.aliases.efforts.extended).toContain("extended");
    expect(message.report.observed_ui.search_mode.status).toBe("not_observed");
    expect(message.report.observed_ui.search_mode.selected).toBe(false);
    expect(message.report.observed_ui.research_flow.status).toBe("observed");
    expect(message.runtime.ui_signature).toContain("aria");
    expect(validateCapabilityReport(message.report)).toBe(true);
  });

  it("detects the current ChatGPT picker and composer-adjacent effort control", () => {
    document.body.dataset.uiVariant = "live";
    document.body.innerHTML = `
      <main aria-label="chatgpt workspace">
        <section class="topbar">
          <button
            type="button"
            data-testid="model-switcher-dropdown-button"
            aria-label="Model selector"
            aria-haspopup="menu"
            aria-expanded="false"
          >
            ChatGPT
          </button>
          <nav aria-label="workspace navigation">
            <button type="button">Search chats</button>
            <a href="/apps" data-testid="apps-button">Apps</a>
            <a href="/deep-research" data-testid="deep-research-sidebar-item">Deep research</a>
            <a href="/codex/cloud">Codex</a>
            <a href="/gpts">GPTs</a>
            <button type="button">Projects</button>
          </nav>
          <button type="button" data-testid="upload-button" aria-label="Add photos and files">Add files</button>
          <input id="live-upload-input" type="file" hidden multiple />
        </section>
        <form data-testid="composer-root">
          <textarea id="prompt-textarea" aria-label="Message ChatGPT" placeholder="Message ChatGPT"></textarea>
          <div class="composer-actions">
            <button
              type="button"
              class="__composer-pill"
              aria-label="Thinking"
              aria-haspopup="menu"
              aria-expanded="false"
            >
              Thinking
            </button>
          </div>
          <button type="button" data-testid="send-button" aria-label="Send prompt">Send</button>
        </form>
        <div data-slot="portal-root">
          <div id="tablinq-model-menu" role="menu" hidden>
            <div role="menuitemradio" data-testid="model-switcher-gpt-5-3" hidden>Instant</div>
            <div role="menuitemradio" data-testid="model-switcher-gpt-5-4-thinking" hidden>Thinking</div>
            <div role="menuitemradio" data-testid="model-switcher-gpt-5-4-pro" hidden>Pro</div>
          </div>
          <div id="tablinq-tools-menu" role="menu">
            <div role="menuitemradio">Web search</div>
          </div>
          <div id="tablinq-effort-menu" role="menu" hidden>
            <div role="menuitemradio" hidden>Standard</div>
            <div role="menuitemradio" hidden>Extended</div>
          </div>
        </div>
        <article data-message-author-role="assistant">
          <div class="markdown">
            <p>Live-like answer text</p>
          </div>
          <a href="/artifact.txt" download data-testid="artifact-download" aria-label="Download report">Download report</a>
        </article>
      </main>
    `;

    const message = probeDocument({
      reportedAt: "2026-04-12T10:00:00Z",
    });

    expect(message.report.matrix).toEqual({
      composer_ready: true,
      submit_available: true,
      model_picker_visible: true,
      thinking_effort_visible: true,
      search_mode_visible: true,
      research_mode_visible: true,
      upload_control_visible: true,
      downloadable_artifacts_observable: true,
    });
    expect(message.runtime.strategy_matches.composer_ready).toBe(
      "chatgpt-prompt-textarea",
    );
    expect(message.runtime.strategy_matches.submit_available).toBe(
      "chatgpt-send-button",
    );
    expect(message.report.aliases.model_classes.instant).toContain("instant");
    expect(message.report.aliases.model_classes.thinking).toContain("thinking");
    expect(message.report.aliases.modes.research).toContain("deep research");
    expect(message.report.aliases.modes.search).toContain("web search");
    expect(message.report.aliases.efforts.extended).toContain("extended");
    expect(message.report.observed_ui.search_mode).toMatchObject({
      status: "observed",
      visible: true,
      selected: false,
    });
    expect(message.report.observed_ui.research_flow).toMatchObject({
      status: "observed",
      entrypoint_visible: true,
      on_research_url: false,
    });
  });

  it("detects the composer-right model picker before the menu opens", () => {
    document.body.dataset.uiVariant = "live";
    document.body.innerHTML = `
      <main aria-label="chatgpt workspace">
        <form data-testid="composer-root">
          <textarea id="prompt-textarea" aria-label="Message ChatGPT"></textarea>
          <button type="button" class="__composer-pill" aria-haspopup="menu" aria-expanded="false">
            5.4 Extended
          </button>
          <button type="button" data-testid="send-button" aria-label="Send prompt">Send</button>
        </form>
      </main>
    `;

    const message = probeDocument({
      reportedAt: "2026-04-12T10:00:00Z",
    });

    expect(message.report.matrix.model_picker_visible).toBe(true);
    expect(message.report.matrix.thinking_effort_visible).toBe(false);
    expect(message.runtime.strategy_matches.model_picker_visible).toBe(
      "chatgpt-model-trigger",
    );
  });

  it("maps the new Intelligence menu Medium and High labels to existing thinking efforts", () => {
    document.body.dataset.uiVariant = "live";
    document.body.innerHTML = `
      <main aria-label="chatgpt workspace">
        <form data-testid="composer-root">
          <textarea id="prompt-textarea" aria-label="Message ChatGPT"></textarea>
          <button type="button" class="__composer-pill" aria-haspopup="menu" aria-expanded="true">
            High
          </button>
          <button type="button" data-testid="send-button" aria-label="Send prompt">Send</button>
        </form>
        <div role="menu">
          <div aria-hidden="true">Intelligence</div>
          <div role="menuitemradio" aria-checked="false">Instant</div>
          <div role="menuitemradio" aria-checked="false">Medium</div>
          <div role="menuitemradio" aria-checked="true">High</div>
          <div role="menuitem">GPT-5.5</div>
        </div>
      </main>
    `;

    const message = probeDocument({
      reportedAt: "2026-04-12T10:00:00Z",
    });

    expect(message.report.matrix.model_picker_visible).toBe(true);
    expect(message.report.matrix.thinking_effort_visible).toBe(true);
    expect(message.report.aliases.model_classes.instant).toContain("instant");
    expect(message.report.aliases.model_classes.thinking).toEqual(
      expect.arrayContaining(["medium", "high"]),
    );
    expect(message.report.aliases.efforts.standard).toContain("medium");
    expect(message.report.aliases.efforts.extended).toContain("high");
    expect(message.report.observed_ui.selected_model).toMatchObject({
      status: "observed",
      selected_model_class: "thinking",
      selected_label: "high",
    });
    expect(message.report.observed_ui.thinking_effort).toMatchObject({
      status: "observed",
      selected_effort: "extended",
    });
  });

  it("detects an active composer Search chip", () => {
    document.body.dataset.uiVariant = "live";
    document.body.innerHTML = `
      <main>
        <form data-testid="composer-root">
          <textarea id="prompt-textarea" aria-label="Message ChatGPT"></textarea>
          <button type="button" class="__composer-pill" aria-label="Search, click to remove">Search</button>
          <button type="button" data-testid="send-button" aria-label="Send prompt">Send</button>
        </form>
      </main>
    `;

    const message = probeDocument({
      reportedAt: "2026-04-12T10:00:00Z",
    });

    expect(message.report.matrix.search_mode_visible).toBe(true);
    expect(message.runtime.strategy_matches.search_mode_visible).toBe(
      "composer-search-chip",
    );
    expect(message.report.aliases.modes.search).toContain(
      "search, click to remove",
    );
    expect(message.report.observed_ui.search_mode).toMatchObject({
      status: "observed",
      visible: true,
      selected: true,
    });
  });

  it("ignores sidebar and conversation search controls", () => {
    document.body.dataset.uiVariant = "live";
    document.body.innerHTML = `
      <main>
        <nav>
          <button type="button">Search chats</button>
          <button type="button" aria-label="Search conversations">Search</button>
        </nav>
        <form data-testid="composer-root">
          <textarea id="prompt-textarea" aria-label="Message ChatGPT"></textarea>
          <button type="button" data-testid="send-button" aria-label="Send prompt">Send</button>
        </form>
      </main>
    `;

    const message = probeDocument({
      reportedAt: "2026-04-12T10:00:00Z",
    });

    expect(message.report.matrix.search_mode_visible).toBe(false);
    expect(message.report.aliases.modes.search).toBeUndefined();
  });

  it("probes semantic drift shapes with generic selector-pack strategies", () => {
    document.body.dataset.uiVariant = "menu";
    document.body.innerHTML = `
      <main aria-label="chatgpt workspace">
        <nav>
          <button type="button" aria-label="Search chats">Search chats</button>
        </nav>
        <form class="stacked-composer">
          <div
            role="textbox"
            aria-label="Message ChatGPT"
            contenteditable="true"
            data-drift-composer="true"
          ></div>
          <button
            type="button"
            id="engine-trigger"
            aria-label="Choose engine"
            aria-haspopup="listbox"
            aria-controls="engine-list"
          >Engine</button>
          <button
            type="button"
            id="depth-trigger"
            aria-label="Reasoning depth"
            aria-haspopup="listbox"
            aria-controls="depth-list"
          >Depth</button>
          <button
            type="button"
            id="tools-trigger"
            aria-label="Add files and more"
            aria-haspopup="listbox"
            aria-controls="tools-list"
          >Tools</button>
          <button type="button" aria-label="Send message">Send</button>
        </form>
        <aside id="semantic-drift-portal-host">
          <div id="engine-list" role="listbox">
            <div role="option" data-model="instant">Instant</div>
            <div role="option" data-model="thinking" aria-selected="true">Thinking</div>
          </div>
          <div id="depth-list" role="listbox">
            <div role="option" data-effort="standard" aria-selected="true">Standard</div>
            <div role="option" data-effort="extended">Extended</div>
          </div>
          <div id="tools-list" role="listbox">
            <div role="option" data-mode="search" aria-label="Browse the web">Browse the web</div>
          </div>
        </aside>
        <article data-message-author-role="assistant">
          <a href="/artifact.txt" download aria-label="Download report">Download report</a>
        </article>
      </main>
    `;

    const message = probeDocument({
      selectorPack: {
        format_version: 1,
        pack_id: "semantic-drift-pack",
        description: "test semantic drift pack",
        updated_at: "2026-04-12T10:00:00Z",
        capabilities: {
          composer_ready: [
            {
              strategy_id: "semantic-drift-contenteditable",
              kind: "css",
              selector: "[data-drift-composer='true']",
              score: 1,
            },
          ],
          search_mode_visible: [
            {
              strategy_id: "semantic-drift-browse-web",
              kind: "text",
              selector: "Browse the web",
              score: 1,
            },
          ],
        },
      } as never,
    });

    expect(message.report.matrix).toMatchObject({
      composer_ready: true,
      submit_available: true,
      model_picker_visible: true,
      thinking_effort_visible: true,
      search_mode_visible: true,
      downloadable_artifacts_observable: true,
    });
    expect(message.runtime.strategy_matches.composer_ready).toBe(
      "semantic-drift-contenteditable",
    );
    expect(message.runtime.strategy_matches.search_mode_visible).toBe(
      "semantic-drift-browse-web",
    );
    expect(message.report.aliases.modes.search).toContain("browse the web");
    expect(message.report.aliases.modes.search).not.toContain("search chats");
    expect(message.report.aliases.model_classes.thinking).toContain("thinking");
  });

  it("keeps selectorless submit probing anchored to the active composer", () => {
    document.body.dataset.uiVariant = "menu";
    document.body.innerHTML = `
      <main aria-label="chatgpt workspace">
        <section id="selectorless-decoys">
          <button type="button" aria-label="Search chats">Search chats</button>
          <button type="button" id="selectorless-stale-send" aria-label="Send prompt">Send</button>
        </section>
        <form class="stacked-composer">
          <div role="textbox" aria-label="Message ChatGPT" contenteditable="true"></div>
          <button
            type="button"
            id="engine-trigger"
            aria-label="Choose engine"
            aria-haspopup="listbox"
            aria-controls="engine-list"
          >Engine</button>
          <button type="button" id="active-send" aria-label="Send message">Send</button>
        </form>
        <div id="engine-list" role="listbox">
          <div role="option" data-model="instant" aria-selected="true">Instant</div>
          <div role="option" data-model="thinking">Thinking</div>
        </div>
      </main>
    `;

    const message = probeDocument({
      selectorPack: {
        format_version: 1,
        pack_id: "selectorless-pack",
        description: "test selectorless pack",
        updated_at: "2026-04-12T10:00:00Z",
        capabilities: {
          submit_available: [
            {
              strategy_id: "selectorless-submit-neighborhood",
              kind: "dom_neighborhood",
              selector: "button[aria-label*='Send']",
              score: 1,
            },
          ],
        },
      } as never,
    });

    const submitNeighborhood = message.runtime.dom_neighborhoods.find(
      (entry) =>
        entry.capability === "submit_available" &&
        entry.strategy_id === "selectorless-submit-neighborhood",
    );
    expect(message.report.matrix.submit_available).toBe(true);
    expect(message.runtime.strategy_matches.submit_available).toBe(
      "selectorless-submit-neighborhood",
    );
    expect(submitNeighborhood?.anchor_path).toContain("button#active-send");
    expect(submitNeighborhood?.anchor_path).not.toContain(
      "selectorless-stale-send",
    );
    expect(message.report.matrix.search_mode_visible).toBe(false);
    expect(message.report.aliases.modes.search).toBeUndefined();
  });

  it("reports missing controls without inventing aliases", () => {
    document.body.innerHTML = `
      <section>
        <div>
          <textarea data-slot="composer" aria-label="Prompt composer"></textarea>
          <button type="button" data-action="submit">Send</button>
        </div>
      </section>
    `;

    const message = probeDocument({
      reportedAt: "2026-04-12T10:00:00Z",
    });

    expect(message.report.matrix.search_mode_visible).toBe(false);
    expect(message.report.matrix.model_picker_visible).toBe(false);
    expect(message.report.aliases.modes.search).toBeUndefined();
    expect(message.report.aliases.model_classes.instant).toBeUndefined();
  });

  it("does not invent model aliases for ambiguous selectorless controls", () => {
    document.body.dataset.uiVariant = "menu";
    document.body.innerHTML = `
      <main>
        <form>
          <div role="textbox" aria-label="Message ChatGPT" contenteditable="true"></div>
          <button type="button" aria-label="Send message">Send</button>
          <button type="button">Options</button>
        </form>
        <section>
          <button type="button">Choice</button>
          <button type="button">Choice</button>
        </section>
      </main>
    `;

    const message = probeDocument({
      reportedAt: "2026-04-12T10:00:00Z",
    });

    expect(message.report.matrix.model_picker_visible).toBe(false);
    expect(message.report.aliases.model_classes.instant).toBeUndefined();
    expect(message.report.aliases.model_classes.thinking).toBeUndefined();
  });

  it("infers instant and thinking aliases from the current ChatGPT trigger before the menu opens", () => {
    document.body.dataset.uiVariant = "menu";
    document.body.innerHTML = `
      <main aria-label="chatgpt workspace">
        <section>
          <button
            type="button"
            data-slot="model-picker"
            aria-label="ChatGPT"
            aria-haspopup="menu"
            aria-expanded="false"
          >
            ChatGPT
          </button>
          <textarea id="prompt-textarea" aria-label="Message ChatGPT"></textarea>
          <button type="button" data-testid="send-button" aria-label="Send prompt">Send</button>
        </section>
      </main>
    `;

    const message = probeDocument({
      reportedAt: "2026-04-12T10:00:00Z",
    });

    expect(message.report.matrix.model_picker_visible).toBe(true);
    expect(message.report.aliases.model_classes.instant).toContain("instant");
    expect(message.report.aliases.model_classes.thinking).toContain("thinking");
    expect(message.runtime.strategy_matches.model_picker_visible).toBeTruthy();
    expect(message.report.observed_ui.selected_model.status).toBe(
      "not_observed",
    );
  });

  it("reports unsupported Pro and ambiguous Latest 5.5 selections without turning them into supported model classes", () => {
    document.body.dataset.uiVariant = "live";
    document.body.innerHTML = `
      <main aria-label="chatgpt workspace">
        <form data-testid="composer-root">
          <textarea id="prompt-textarea" aria-label="Message ChatGPT"></textarea>
          <button type="button" class="__composer-pill" aria-haspopup="menu" aria-expanded="false">
            ChatGPT Pro
          </button>
          <button type="button" data-testid="send-button" aria-label="Send prompt">Send</button>
        </form>
      </main>
    `;

    const proMessage = probeDocument({
      reportedAt: "2026-04-12T10:00:00Z",
    });

    expect(proMessage.report.observed_ui.selected_model).toMatchObject({
      status: "blocked",
      selected_model_class: null,
      unsupported_pro_selected: true,
      ambiguous_selection: false,
    });

    document.body.innerHTML = `
      <main aria-label="chatgpt workspace">
        <form data-testid="composer-root">
          <textarea id="prompt-textarea" aria-label="Message ChatGPT"></textarea>
          <button type="button" class="__composer-pill" aria-haspopup="menu" aria-expanded="false">
            Latest 5.5
          </button>
          <button type="button" data-testid="send-button" aria-label="Send prompt">Send</button>
        </form>
      </main>
    `;

    const ambiguousMessage = probeDocument({
      reportedAt: "2026-04-12T10:00:00Z",
    });

    expect(ambiguousMessage.report.observed_ui.selected_model).toMatchObject({
      status: "unknown",
      selected_model_class: null,
      unsupported_pro_selected: false,
      ambiguous_selection: true,
    });
  });

  it("accepts selector-pack driven strategy ordering metadata", () => {
    document.body.innerHTML = `
      <section>
        <div>
          <textarea data-slot="composer" aria-label="Prompt composer"></textarea>
          <button type="button" data-action="submit">Send</button>
        </div>
      </section>
    `;

    const message = probeDocument({
      selectorPackId: "custom-pack",
      selectorPackVersion: "v1",
      selectorPack: {
        format_version: 1,
        pack_id: "custom-pack",
        description: "test pack",
        updated_at: "2026-04-12T10:00:00Z",
        capabilities: {
          composer_ready: [
            {
              strategy_id: "aria-textbox",
              kind: "aria_role",
              selector: "[role='textbox']",
              score: 1,
            },
            {
              strategy_id: "data-slot-composer",
              kind: "css",
              selector: "[data-slot='composer']",
              score: 0.5,
            },
          ],
        },
      } as never,
    });

    expect(message.report.selector_pack_id).toBe("custom-pack");
    expect(message.runtime.strategy_matches.composer_ready).toBe(
      "aria-textbox",
    );
  });

  it("executes unknown selector-pack strategies generically for known capabilities", () => {
    document.body.innerHTML = `
      <section>
        <div role="textbox" aria-label="Message ChatGPT" contenteditable="true"></div>
        <button type="button" aria-label="Launch message" data-custom-submit>Launch</button>
      </section>
    `;

    const message = probeDocument({
      selectorPack: {
        format_version: 1,
        pack_id: "custom-pack",
        description: "test pack",
        updated_at: "2026-04-12T10:00:00Z",
        capabilities: {
          submit_available: [
            {
              strategy_id: "custom-submit-control",
              kind: "css",
              selector: "[data-custom-submit]",
              score: 1,
            },
          ],
        },
      } as never,
    });

    expect(message.report.matrix.submit_available).toBe(true);
    expect(message.runtime.strategy_matches.submit_available).toBe(
      "custom-submit-control",
    );
  });

  it("keeps current stable strategies ahead of lower-score generic selector-pack entries", () => {
    document.body.innerHTML = `
      <section>
        <textarea data-slot="composer" aria-label="Prompt composer"></textarea>
        <div role="textbox" aria-label="Message ChatGPT" contenteditable="true"></div>
      </section>
    `;

    const message = probeDocument({
      selectorPack: {
        format_version: 1,
        pack_id: "custom-pack",
        description: "test pack",
        updated_at: "2026-04-12T10:00:00Z",
        capabilities: {
          composer_ready: [
            {
              strategy_id: "custom-contenteditable-composer",
              kind: "css",
              selector: "[contenteditable='true']",
              score: 0.5,
            },
          ],
        },
      } as never,
    });

    expect(message.runtime.strategy_matches.composer_ready).toBe(
      "data-slot-composer",
    );
  });

  it("lets preferred strategies outrank selector-pack generic reordering", () => {
    document.body.innerHTML = `
      <section>
        <textarea data-slot="composer" aria-label="Prompt composer"></textarea>
        <button type="button" data-action="submit">Send</button>
        <button type="button" aria-label="Launch message" data-custom-submit>Launch</button>
      </section>
    `;

    const message = probeDocument({
      preferredStrategies: { submit_available: "data-action-submit" },
      selectorPack: {
        format_version: 1,
        pack_id: "custom-pack",
        description: "test pack",
        updated_at: "2026-04-12T10:00:00Z",
        capabilities: {
          submit_available: [
            {
              strategy_id: "custom-submit-control",
              kind: "css",
              selector: "[data-custom-submit]",
              score: 1,
            },
          ],
        },
      } as never,
    });

    expect(message.runtime.strategy_matches.submit_available).toBe(
      "data-action-submit",
    );
  });
});

describe("service worker bridge", () => {
  it("records missing native host errors without throwing", async () => {
    const controller = createBridgeController({
      transport: "native_messaging",
      chrome: {
        runtime: {
          sendNativeMessage: async () => {
            throw new Error("Specified native messaging host not found.");
          },
        },
      },
    });

    document.body.innerHTML = `
      <textarea data-slot="composer" aria-label="Prompt composer"></textarea>
      <button type="button" data-action="submit">Send</button>
    `;
    const message = probeDocument({
      reportedAt: "2026-04-12T10:00:00Z",
    });

    await expect(controller.handleMessage(message)).resolves.toBeUndefined();
    expect(controller.describe()).toMatchObject({
      connected: false,
      last_error: "Specified native messaging host not found.",
      transport: "native_messaging",
    });
  });

  it("forwards capability reports through the development websocket bridge", async () => {
    let sentPayload = "";
    const statusUpdates: BridgeStatus[] = [];
    const controller = createBridgeController({
      transport: "websocket_dev",
      webSocketUrl: "ws://127.0.0.1:4319",
      createWebSocket: () =>
        ({
          readyState: WebSocket.OPEN,
          send(payload: string) {
            sentPayload = payload;
          },
          addEventListener() {},
        }) as unknown as WebSocket,
    });

    document.body.innerHTML = `
      <textarea data-slot="composer" aria-label="Prompt composer"></textarea>
      <button type="button" data-action="submit">Send</button>
    `;
    const message = probeDocument({
      reportedAt: "2026-04-12T10:00:00Z",
    });

    await controller.handleMessage(message, {
      tab: {
        id: 17,
        url: "https://chatgpt.com/c/conv-9",
      },
    });
    statusUpdates.push(controller.describe());

    expect(JSON.parse(sentPayload)).toMatchObject({
      type: "capability_report",
      report: {
        reported_at: "2026-04-12T10:00:00Z",
      },
      tab: {
        id: 17,
        url: "https://chatgpt.com/c/conv-9",
        workspace_key: "https://chatgpt.com/",
      },
    });
    expect(statusUpdates[0].connected).toBe(true);
    expect(statusUpdates[0].transport).toBe("websocket_dev");
  });
});
