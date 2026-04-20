// @vitest-environment jsdom

import addFormats from "ajv-formats";
import Ajv2020 from "ajv/dist/2020";
import { loadSchema } from "../../browser-adapter/src/protocol";
import { probeDocument } from "../src/probe";
import {
  createBridgeController,
  type BridgeStatus,
} from "../src/service-worker";

const ajv = new Ajv2020({ allErrors: true, strict: false });
addFormats(ajv);

const validateCapabilityReport = ajv.compile(
  loadSchema<Record<string, unknown>>("capability-report"),
);

describe("extension capability probe", () => {
  beforeEach(() => {
    document.body.innerHTML = "";
    document.body.dataset.uiVariant = "classic";
  });

  it("detects the classic fake harness controls", () => {
    document.body.innerHTML = `
      <section>
        <div>
          <div data-slot="model-picker" role="group" aria-label="Model picker">
            <button type="button" data-model="instant" aria-pressed="false">GPT-5.3 Instant</button>
            <button type="button" data-model="thinking" aria-pressed="true">GPT-5.4 Thinking</button>
          </div>
          <button type="button" data-mode="search">Search</button>
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
      search_mode_visible: true,
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
    expect(message.report.aliases.model_classes.thinking).toContain(
      "thinking",
    );
    expect(message.report.aliases.efforts.extended).toContain("extended");
    expect(message.report.aliases.modes.research).toContain("deep research");
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
          <button type="button" aria-pressed="false">Browse the web</button>
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
      search_mode_visible: true,
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
    expect(message.runtime.strategy_matches.search_mode_visible).toBe(
      "chatgpt-search-toggle",
    );
    expect(message.report.aliases.modes.search).toContain("browse the web");
    expect(message.report.aliases.modes.research).toContain("deep research");
    expect(message.report.aliases.model_classes.instant).toContain("instant");
    expect(message.report.aliases.model_classes.thinking).toContain(
      "thinking",
    );
    expect(message.report.aliases.efforts.extended).toContain("extended");
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
          <button type="button" data-mode="search" aria-label="Search">Search</button>
          <nav aria-label="workspace navigation">
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
    expect(message.report.aliases.efforts.extended).toContain("extended");
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
