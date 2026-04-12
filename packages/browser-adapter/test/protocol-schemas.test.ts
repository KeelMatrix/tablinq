import addFormats from "ajv-formats";
import Ajv2020 from "ajv/dist/2020";
import { loadSchema, type ProtocolSchemaName } from "../src/protocol";

const ajv = new Ajv2020({ allErrors: true, strict: false });
addFormats(ajv);

function expectValid(name: ProtocolSchemaName, sample: unknown) {
  const validate = validatorFor(name);
  const isValid = validate(sample);
  expect(validate.errors).toBeNull();
  expect(isValid).toBe(true);
}

function validatorFor(name: ProtocolSchemaName) {
  const schema = loadSchema<Record<string, unknown>>(name);
  const schemaId = typeof schema.$id === "string" ? schema.$id : name;
  return ajv.getSchema(schemaId) ?? ajv.compile(schema);
}

describe("protocol schemas", () => {
  it("accepts the command envelope sample", () => {
    expectValid("commands", {
      version: "tablinq.v1",
      request_id: "req-001",
      session_id: "sess-001",
      sent_at: "2026-04-12T10:00:00Z",
      command: {
        type: "ask",
        payload: {
          prompt: "hello",
          conversation_id: null,
          mode: "search",
          model_class: "thinking",
          effort: "standard",
          files: ["fixtures/input.pdf"],
        },
      },
    });
  });

  it("accepts the event envelope sample", () => {
    expectValid("events", {
      version: "tablinq.v1",
      request_id: "req-001",
      emitted_at: "2026-04-12T10:00:01Z",
      event: {
        type: "capability_report",
        payload: {
          reported_at: "2026-04-12T10:00:01Z",
          selector_pack_id: "chatgpt-default",
          selector_pack_version: "v1",
          matrix: {
            composer_ready: true,
            submit_available: true,
            model_picker_visible: true,
            thinking_effort_visible: true,
            search_mode_visible: true,
            research_mode_visible: false,
            upload_control_visible: true,
            downloadable_artifacts_observable: false,
          },
          aliases: {
            modes: {
              normal: ["Chat"],
              search: ["Search"],
            },
            model_classes: {
              instant: ["GPT-5.3 Instant"],
              thinking: ["GPT-5.4 Thinking"],
            },
            efforts: {
              standard: ["Standard"],
              extended: ["Extended"],
            },
          },
        },
      },
    });
  });

  it("accepts the stream chunk sample", () => {
    expectValid("stream-chunk", {
      job_id: "job-001",
      conversation_id: "conv-001",
      message_id: "msg-001",
      seq: 2,
      kind: "delta",
      text: "partial output",
    });
  });

  it("accepts the capability report sample", () => {
    expectValid("capability-report", {
      reported_at: "2026-04-12T10:00:01Z",
      selector_pack_id: "chatgpt-default",
      selector_pack_version: "v1",
      matrix: {
        composer_ready: true,
        submit_available: true,
        model_picker_visible: true,
        thinking_effort_visible: true,
        search_mode_visible: true,
        research_mode_visible: true,
        upload_control_visible: true,
        downloadable_artifacts_observable: false,
      },
      aliases: {
        modes: {
          normal: ["Chat"],
        },
        model_classes: {
          instant: ["GPT-5.3 Instant"],
        },
        efforts: {
          none: ["None"],
        },
      },
    });
  });

  it("accepts the job state sample", () => {
    expectValid("job-state", {
      job_id: "job-001",
      status: "running",
      message: "Waiting for streamed chunks",
      conversation_id: "conv-001",
      updated_at: "2026-04-12T10:05:00Z",
      artifacts: [],
    });
  });

  it("accepts the artifact metadata sample", () => {
    expectValid("artifact-metadata", {
      artifact_id: "artifact-001",
      conversation_id: "conv-001",
      message_id: "msg-001",
      file_name: "report.md",
      content_type: "text/markdown",
      size_bytes: 1024,
      sha256: "abc123",
      storage_path: "downloads/report.md",
    });
  });

  it("accepts the failure bundle sample", () => {
    expectValid("failure-bundle", {
      bundle_id: "bundle-001",
      error_code: "ui_drift",
      message: "Model picker probe failed",
      created_at: "2026-04-12T10:06:00Z",
      url: "https://chatgpt.com",
      mode: "search",
      screenshot_path: "bundles/failure.png",
      dom_snapshot_path: "bundles/dom.html",
      aria_snapshot_path: "bundles/aria.json",
      logs_path: "logs/gptd.log",
      strategy_attempts: [
        {
          capability: "model_picker_visible",
          strategy_id: "toolbar-button",
          score: 0.91,
          outcome: "miss",
          detail: "No toolbar button matched accessible name",
        },
      ],
      capability_matrix: {
        composer_ready: true,
        submit_available: true,
        model_picker_visible: false,
        thinking_effort_visible: false,
        search_mode_visible: true,
        research_mode_visible: false,
        upload_control_visible: true,
        downloadable_artifacts_observable: false,
      },
    });
  });

  it("accepts the health check sample", () => {
    expectValid("health-check", {
      status: "ok",
      checked_at: "2026-04-12T10:07:00Z",
      browser_connected: true,
      extension_connected: true,
      authenticated: true,
      details: "profile warm",
    });
  });

  it("rejects malformed commands", () => {
    const validate = validatorFor("commands");
    const isValid = validate({
      version: "tablinq.v1",
      sent_at: "2026-04-12T10:00:00Z",
      command: {
        type: "health_ping",
        payload: {},
      },
    });

    expect(isValid).toBe(false);
    expect(validate.errors?.length).toBeGreaterThan(0);
  });
});
