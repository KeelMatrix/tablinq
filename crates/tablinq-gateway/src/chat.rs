mod execution;
mod interruption;
mod job_execution;
mod preparation;
mod response;
mod sse_writer;

use chrono::{DateTime, Utc};
use gptd::browser::execution_error_metadata;
use shared_protocol::{
    JobStatus, RecoveryPacket, RecoveryTrace, RequestExecutionKind, RequestSummary, RequestUsage,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{HttpRequest, RequestContext};
    use crate::openai::{ChatCompletionsRequest, RequestRole};
    use chrono::Utc;
    use gptd::Database;
    use gptd::core::PreparedAskJob;
    use serde_json::{Value, json};
    use shared_protocol::{
        AskCommand, ErrorCode, JobStatus, Mode, RecoveryStrategy, RequestExecutionKind,
    };
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn prepared_auto_recover_ask() -> PreparedChatRequest {
        let request: ChatCompletionsRequest = serde_json::from_value(json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "normal success" }],
            "tablinq_auto_recover_provider_generation_failure": true
        }))
        .unwrap();
        PreparedChatRequest {
            request,
            model_id: "tablinq-instant",
            request_role: RequestRole::Primary,
            parent_request_id: None,
            file_ids: vec![],
            job: PreparedChatJob::Ask(PreparedAskJob {
                job_id: "job-normal-success".to_owned(),
                command: AskCommand {
                    prompt: "normal success".to_owned(),
                    conversation_id: None,
                    new_conversation: false,
                    mode: Mode::Normal,
                    model_class: None,
                    effort: None,
                    files: vec![],
                    prompt_upload_threshold_chars: None,
                    prompt_upload_file_name: None,
                    prompt_upload_composer_text: None,
                    auto_recover_provider_generation_failure: true,
                },
                created_at: Utc::now(),
            }),
        }
    }

    fn request_context(request_id: &str) -> RequestContext {
        RequestContext {
            request_id: request_id.to_owned(),
            request: HttpRequest {
                method: "POST".to_owned(),
                path: "/v1/chat/completions".to_owned(),
                headers: BTreeMap::new(),
                body: vec![],
            },
        }
    }

    #[test]
    fn auto_recover_enabled_normal_success_surfaces_completed_status() {
        let context = request_context("req-auto-recover-normal");
        let prepared = prepared_auto_recover_ask();
        let now = Utc::now();
        let outcome = RequestOutcome {
            kind: RequestOutcomeKind::Completed,
            low_confidence: true,
            conversation_id: Some("conv-normal".to_owned()),
            interrupted_at: None,
            interruption_reason: None,
            recovered_at: None,
            recovery_reason: None,
            failed_at: None,
            failure_reason: None,
            recovery_trace: None,
            recovery_packet: None,
        };
        let summary = build_request_summary(&context, &prepared, now, now, 0, outcome);

        assert_eq!(summary.status, JobStatus::Completed);
        assert_eq!(summary.execution_kind, RequestExecutionKind::Ask);
        assert!(summary.recovered_at.is_none());
        assert!(summary.recovery_reason.is_none());
        assert!(summary.recovery_trace.is_none());
        assert!(summary.recovery_packet.is_none());

        let dir = tempdir().unwrap();
        let db = Database::open(&dir.path().join("tablinq.sqlite3")).unwrap();
        db.upsert_request_summary(&summary).unwrap();

        let detail = db
            .request_summary("req-auto-recover-normal")
            .unwrap()
            .unwrap();
        assert_eq!(detail.status, JobStatus::Completed);
        assert!(detail.recovered_at.is_none());
        assert!(detail.recovery_reason.is_none());
        assert!(detail.recovery_trace.is_none());
        assert!(detail.recovery_packet.is_none());

        let list = db.list_request_page(None, 10, 0).unwrap();
        assert_eq!(list.requests[0].id, "req-auto-recover-normal");
        assert_eq!(list.requests[0].status, JobStatus::Completed);

        let usage_history = db.list_request_summaries(10).unwrap();
        assert_eq!(
            usage_history.requests[0].request_id,
            "req-auto-recover-normal"
        );
        assert_eq!(usage_history.requests[0].status, JobStatus::Completed);
    }

    #[test]
    fn direct_response_recovery_fields_match_completed_stored_status() {
        let response = response::build_chat_completion_response(
            "req-direct-normal",
            "job-direct-normal",
            "tablinq-instant",
            RequestRole::Primary,
            None,
            Some("conv-normal"),
            Some("session-normal"),
            "done",
            "stop",
            None,
        );
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["tablinq_recovery_available"], false);
        assert_eq!(body["tablinq_recovery_attempted"], false);
        assert_eq!(body["tablinq_recovery_strategy"], Value::Null);
        assert_eq!(body["tablinq_replacement_request_id"], Value::Null);
        assert_eq!(body["tablinq_replacement_conversation_id"], Value::Null);

        let context = request_context("req-direct-normal");
        let prepared = prepared_auto_recover_ask();
        let now = Utc::now();
        let summary = build_request_summary(
            &context,
            &prepared,
            now,
            now,
            0,
            RequestOutcome {
                kind: RequestOutcomeKind::Completed,
                low_confidence: false,
                conversation_id: Some("conv-normal".to_owned()),
                interrupted_at: None,
                interruption_reason: None,
                recovered_at: None,
                recovery_reason: None,
                failed_at: None,
                failure_reason: None,
                recovery_trace: None,
                recovery_packet: None,
            },
        );
        assert_eq!(summary.status, JobStatus::Completed);
    }

    #[test]
    fn attempted_provider_recovery_summary_remains_recovered() {
        let trace = RecoveryTrace {
            recovery_available: true,
            recovery_attempted: true,
            recovery_strategy: Some(RecoveryStrategy::ReloadNewConversationReplayWithContext),
            failure_code: Some(ErrorCode::ProviderGenerationFailedNonretryable),
            failure_bundle_id: Some("bundle-provider".to_owned()),
            prior_request_id: Some("job-provider:provider-attempt-0".to_owned()),
            replacement_request_id: Some("job-provider:provider-attempt-1".to_owned()),
            prior_conversation_id: Some("conv-old".to_owned()),
            replacement_conversation_id: Some("conv-new".to_owned()),
            recovery_attempt_count: 1,
        };
        let context = request_context("req-provider-recovered");
        let mut prepared = prepared_auto_recover_ask();
        if let PreparedChatJob::Ask(job) = &mut prepared.job {
            job.job_id = "job-provider".to_owned();
        }
        let now = Utc::now();
        let summary = build_request_summary(
            &context,
            &prepared,
            now,
            now,
            0,
            RequestOutcome {
                kind: RequestOutcomeKind::Recovered,
                low_confidence: true,
                conversation_id: Some("conv-new".to_owned()),
                interrupted_at: None,
                interruption_reason: None,
                recovered_at: Some(now),
                recovery_reason: Some(
                    "provider generation failure recovered via reload_new_conversation_replay_with_context"
                        .to_owned(),
                ),
                failed_at: None,
                failure_reason: None,
                recovery_trace: Some(trace),
                recovery_packet: None,
            },
        );

        assert_eq!(summary.status, JobStatus::Recovered);
        assert!(summary.recovered_at.is_some());
        assert!(summary.recovery_reason.is_some());
        assert!(
            summary
                .recovery_trace
                .as_ref()
                .is_some_and(|trace| trace.recovery_attempted)
        );
    }
}
