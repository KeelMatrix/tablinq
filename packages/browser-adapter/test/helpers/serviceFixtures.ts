import type { BrowserStatusSnapshot } from "../../src/browserAdapter";
import type {
  AdapterFrame,
  RecoverAskRequest,
  RecoverResearchRequest,
} from "../../src/service/protocol";

export function statusSnapshotFixture(
  overrides: Partial<BrowserStatusSnapshot> = {},
): BrowserStatusSnapshot {
  return {
    running: false,
    browser_kind: "chromium",
    profile_dir: "profile",
    downloads_dir: "downloads",
    target_url: "https://chatgpt.test",
    current_url: null,
    page_title: null,
    page_ready: false,
    authenticated: false,
    reload_count: 0,
    screenshot_path: null,
    browser_path: null,
    extension_connected: false,
    capability_report: null,
    attach_mode: "managed_only",
    browser_owner: "managed",
    cdp_attached: false,
    target_tab_found: false,
    target_tab_id: null,
    target_workspace_key: null,
    debug_endpoint: "http://127.0.0.1:9222",
    auth_source: "unknown",
    extension_required: true,
    last_capability_report_at: null,
    ...overrides,
  };
}

export function recoverAskRequestFixture(
  overrides: Partial<RecoverAskRequest> = {},
): RecoverAskRequest {
  return {
    type: "recover_ask",
    job_id: "job-recover",
    prompt: "resume",
    emitted_text: null,
    next_seq: null,
    baseline_assistant_message_count: 0,
    baseline_conversation_id: null,
    baseline_message_id: "msg-base",
    baseline_text: null,
    ...overrides,
  };
}

export function recoverResearchRequestFixture(
  overrides: Partial<RecoverResearchRequest> = {},
): RecoverResearchRequest {
  return {
    type: "recover_research",
    job_id: "job-research-recover",
    query: "resume research",
    emitted_text: null,
    next_seq: null,
    ...overrides,
  };
}

export function captureAdapterFrames() {
  const frames: AdapterFrame[] = [];
  return {
    frames,
    write(frame: AdapterFrame) {
      frames.push(frame);
    },
  };
}
