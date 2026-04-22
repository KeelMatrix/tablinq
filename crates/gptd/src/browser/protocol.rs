use super::{BrowserDiagnosticsSnapshot, RequestLifecycleUpdate};
use serde::{Deserialize, Serialize};
use shared_protocol::{
    AskResult, BrowserStatusSnapshot, Effort, JobStatus, Mode, ModelClass, ResearchResult,
    StreamChunkKind,
};

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum AdapterRequest {
    Open {
        target_url: String,
        browser_path: Option<String>,
        headless: bool,
        force_reload: bool,
    },
    Health,
    Ask {
        job_id: String,
        prompt: String,
        conversation_id: Option<String>,
        new_conversation: bool,
        timeout_ms: Option<u64>,
        mode: Option<Mode>,
        model_class: Option<ModelClass>,
        effort: Option<Effort>,
        files: Vec<String>,
        emitted_text: Option<String>,
        next_seq: Option<u64>,
    },
    RecoverAsk {
        job_id: String,
        prompt: String,
        conversation_id: Option<String>,
        new_conversation: bool,
        timeout_ms: Option<u64>,
        mode: Option<Mode>,
        model_class: Option<ModelClass>,
        effort: Option<Effort>,
        files: Vec<String>,
        emitted_text: Option<String>,
        next_seq: Option<u64>,
    },
    Research {
        job_id: String,
        query: String,
        conversation_id: Option<String>,
        files: Vec<String>,
        output_dir: Option<String>,
        timeout_ms: Option<u64>,
        emitted_text: Option<String>,
        next_seq: Option<u64>,
    },
    RecoverResearch {
        job_id: String,
        query: String,
        conversation_id: Option<String>,
        files: Vec<String>,
        output_dir: Option<String>,
        timeout_ms: Option<u64>,
        emitted_text: Option<String>,
        next_seq: Option<u64>,
    },
    Diagnostics {
        bundle_dir: Option<String>,
    },
    Close,
    Shutdown,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(super) enum AdapterFrame {
    Response {
        ok: bool,
        state: BrowserStatusSnapshot,
        error: Option<String>,
        error_code: Option<String>,
    },
    StreamChunk {
        chunk: AdapterStreamChunk,
    },
    RequestState {
        update: AdapterRequestStateUpdate,
    },
    AskResult {
        ok: bool,
        state: BrowserStatusSnapshot,
        result: Option<AskResult>,
        error: Option<String>,
    },
    ResearchUpdate {
        update: AdapterResearchUpdate,
    },
    ResearchResult {
        ok: bool,
        state: BrowserStatusSnapshot,
        result: Option<ResearchResult>,
        error: Option<String>,
    },
    Diagnostics {
        ok: bool,
        state: BrowserStatusSnapshot,
        diagnostics: Option<BrowserDiagnosticsSnapshot>,
        error: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AdapterStreamChunk {
    pub job_id: String,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub seq: u64,
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AdapterResearchUpdate {
    pub status: JobStatus,
    pub message: Option<String>,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub current_url: Option<String>,
    pub chunk_kind: Option<StreamChunkKind>,
    pub chunk_text: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AdapterRequestStateUpdate {
    pub state: String,
    pub reason: String,
    pub at: String,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub current_url: Option<String>,
    pub streaming_safe: bool,
}

pub(super) fn map_request_state_update(
    update: AdapterRequestStateUpdate,
) -> RequestLifecycleUpdate {
    let status = match update.state.as_str() {
        "recovered" => JobStatus::Recovered,
        _ => JobStatus::Interrupted,
    };
    RequestLifecycleUpdate {
        status,
        message: if update.at.is_empty() {
            update.reason
        } else {
            format!("{} ({})", update.reason, update.at)
        },
        conversation_id: update.conversation_id,
        message_id: update.message_id,
        current_url: update.current_url,
        streaming_safe: update.streaming_safe,
    }
}
