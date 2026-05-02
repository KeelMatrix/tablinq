mod execution;
mod interruption;
mod job_execution;
mod preparation;
mod response;
mod sse_writer;

use chrono::{DateTime, Utc};
use gptd::browser::execution_error_metadata;
use shared_protocol::{
    EventPayload, JobStatus, RecoveryPacket, RecoveryTrace, RequestExecutionKind, RequestSummary,
    RequestUsage,
};

pub(crate) use execution::execute_chat_completions_request;
pub(crate) use preparation::{
    PreparedChatJob, PreparedChatRequest, prepare_chat_completions_request,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestOutcomeKind {
    Completed,
    Recovered,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub(crate) struct RequestOutcome {
    pub(crate) kind: RequestOutcomeKind,
    pub(crate) low_confidence: bool,
    pub(crate) conversation_id: Option<String>,
    pub(crate) interrupted_at: Option<DateTime<Utc>>,
    pub(crate) interruption_reason: Option<String>,
    pub(crate) recovered_at: Option<DateTime<Utc>>,
    pub(crate) recovery_reason: Option<String>,
    pub(crate) failed_at: Option<DateTime<Utc>>,
    pub(crate) failure_reason: Option<String>,
    pub(crate) recovery_trace: Option<RecoveryTrace>,
    pub(crate) recovery_packet: Option<RecoveryPacket>,
}

pub(crate) fn build_request_summary(
    context: &crate::http::RequestContext,
    prepared: &PreparedChatRequest,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    queue_wait_ms: u64,
    outcome: RequestOutcome,
) -> RequestSummary {
    RequestSummary {
        request_id: context.request_id.clone(),
        job_id: prepared.job_id().to_owned(),
        model_id: Some(prepared.model_id.to_owned()),
        request_role: Some(crate::openai::request_role_name(prepared.request_role).to_owned()),
        parent_request_id: prepared.parent_request_id.clone(),
        execution_kind: match &prepared.job {
            PreparedChatJob::Ask(_) => RequestExecutionKind::Ask,
            PreparedChatJob::Research(_) => RequestExecutionKind::Research,
        },
        status: match outcome.kind {
            RequestOutcomeKind::Completed => JobStatus::Completed,
            RequestOutcomeKind::Recovered => JobStatus::Recovered,
            RequestOutcomeKind::Failed => JobStatus::Failed,
            RequestOutcomeKind::Cancelled => JobStatus::Cancelled,
        },
        cancelled: outcome.kind == RequestOutcomeKind::Cancelled,
        start_time,
        end_time: Some(end_time),
        duration_ms: Some((end_time - start_time).num_milliseconds().max(0) as u64),
        queue_wait_ms,
        interrupted_at: outcome.interrupted_at,
        interruption_reason: outcome.interruption_reason,
        recovered_at: outcome.recovered_at,
        recovery_reason: outcome.recovery_reason,
        failed_at: outcome.failed_at,
        failure_reason: outcome.failure_reason,
        recovery_trace: outcome.recovery_trace,
        recovery_packet: outcome.recovery_packet,
        usage: RequestUsage {
            status: "unknown".to_owned(),
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
        },
        warnings: vec![
            "exact token usage is unavailable for the browser-backed provider".to_owned(),
        ],
    }
}

pub(crate) fn outcome_from_error(err: &anyhow::Error) -> RequestOutcome {
    let metadata = execution_error_metadata(err);
    let message = metadata
        .as_ref()
        .map(|item| item.message.clone())
        .unwrap_or_else(|| format!("{err:#}"));
    RequestOutcome {
        kind: if message.contains("cancelled") {
            RequestOutcomeKind::Cancelled
        } else {
            RequestOutcomeKind::Failed
        },
        low_confidence: message.contains("timed out"),
        conversation_id: None,
        interrupted_at: None,
        interruption_reason: None,
        recovered_at: None,
        recovery_reason: None,
        failed_at: if message.contains("cancelled") {
            None
        } else {
            Some(Utc::now())
        },
        failure_reason: if message.contains("cancelled") {
            None
        } else {
            Some(message)
        },
        recovery_trace: metadata
            .as_ref()
            .and_then(|item| item.recovery_trace.clone()),
        recovery_packet: metadata.and_then(|item| item.recovery_packet),
    }
}

fn event_conversation_id(event: &EventPayload) -> Option<String> {
    match event {
        EventPayload::StreamChunk(chunk) => chunk.conversation_id.clone(),
        EventPayload::JobState(state) => state.conversation_id.clone(),
        EventPayload::ArtifactReady(artifact) => artifact.conversation_id.clone(),
        _ => None,
    }
}
