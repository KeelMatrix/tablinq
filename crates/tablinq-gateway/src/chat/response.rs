use crate::http::HttpResponse;
use crate::json::json_response;
use crate::openai::{RequestRole, request_role_name};
use chrono::Utc;
use serde_json::{Value, json};
use shared_protocol::RecoveryTrace;

#[allow(clippy::too_many_arguments)]
pub(super) fn build_chat_completion_response(
    request_id: &str,
    job_id: &str,
    model_id: &str,
    request_role: RequestRole,
    parent_request_id: Option<&str>,
    conversation_id: Option<&str>,
    session_id: Option<&str>,
    content: &str,
    finish_reason: &str,
    recovery_trace: Option<&RecoveryTrace>,
) -> HttpResponse {
    json_response(
        200,
        json!({
            "id": format!("chatcmpl-{}", Utc::now().timestamp_millis()),
            "object": "chat.completion",
            "created": Utc::now().timestamp(),
            "model": model_id,
            "request_id": request_id,
            "tablinq_job_id": job_id,
            "tablinq_request_role": request_role_name(request_role),
            "tablinq_parent_request_id": parent_request_id,
            "tablinq_conversation_id": conversation_id,
            "tablinq_session_id": session_id,
            "tablinq_recovery_available": recovery_trace.map(|trace| trace.recovery_available).unwrap_or(false),
            "tablinq_recovery_attempted": recovery_trace.map(|trace| trace.recovery_attempted).unwrap_or(false),
            "tablinq_recovery_strategy": recovery_trace.and_then(|trace| trace.recovery_strategy.as_ref()).map(recovery_strategy_name),
            "tablinq_prior_request_id": recovery_trace.and_then(|trace| trace.prior_request_id.as_deref()),
            "tablinq_replacement_request_id": recovery_trace.and_then(|trace| trace.replacement_request_id.as_deref()),
            "tablinq_replacement_conversation_id": recovery_trace.and_then(|trace| trace.replacement_conversation_id.as_deref()),
            "tablinq_failure_bundle_id": recovery_trace.and_then(|trace| trace.failure_bundle_id.as_deref()),
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content,
                },
                "finish_reason": finish_reason,
            }],
            "usage": Value::Null,
            "warnings": ["usage is unavailable for the browser-backed provider"],
        }),
    )
}

fn recovery_strategy_name(strategy: &shared_protocol::RecoveryStrategy) -> &'static str {
    match strategy {
        shared_protocol::RecoveryStrategy::ReloadNewConversationReplayWithContext => {
            "reload_new_conversation_replay_with_context"
        }
    }
}

pub(super) fn recovery_reason_for_trace(trace: &RecoveryTrace) -> Option<String> {
    trace
        .recovery_strategy
        .as_ref()
        .map(recovery_strategy_name)
        .map(|strategy| format!("provider generation failure recovered via {strategy}"))
}
