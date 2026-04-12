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
          <button type="button" data-slot="model-picker" aria-haspopup="listbox">
            GPT-5.4 Thinking
          </button>
          <button type="button" data-mode="search">Search</button>
          <button type="button" data-mode="research">Research</button>
          <button type="button" data-slot="effort-toggle">Standard</button>
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
    expect(message.runtime.dom_neighborhoods.length).toBeGreaterThan(3);
    expect(validateCapabilityReport(message.report)).toBe(true);
    expect(validateCapabilityReport.errors).toBeNull();
  });

  it("falls back to aria and neighborhood strategies on the alternate UI", () => {
    document.body.dataset.uiVariant = "aria";
    document.body.innerHTML = `
      <section aria-label="assistant workspace">
        <div>
          <button type="button" aria-haspopup="listbox">
            Choose engine: GPT-5.3 Instant / GPT-5.4 Thinking
          </button>
          <button type="button" aria-pressed="false">Browse the web</button>
          <button type="button" aria-pressed="false">Deep research</button>
          <button type="button" aria-label="Reasoning effort">
            Extended reasoning
          </button>
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
    expect(message.runtime.strategy_matches.search_mode_visible).toBe(
      "aria-search-toggle",
    );
    expect(message.report.aliases.model_classes.instant).toContain(
      "choose engine: gpt-5.3 instant / gpt-5.4 thinking",
    );
    expect(message.report.aliases.modes.search).toContain("browse the web");
    expect(message.runtime.ui_signature).toContain("aria");
    expect(validateCapabilityReport(message.report)).toBe(true);
  });
});

describe("service worker bridge", () => {
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

    await controller.handleMessage(message);
    statusUpdates.push(controller.describe());

    expect(JSON.parse(sentPayload)).toMatchObject({
      type: "capability_report",
      report: {
        reported_at: "2026-04-12T10:00:00Z",
      },
    });
    expect(statusUpdates[0].connected).toBe(true);
    expect(statusUpdates[0].transport).toBe("websocket_dev");
  });
});
