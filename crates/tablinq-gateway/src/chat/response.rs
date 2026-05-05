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

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_assistant_text() -> &'static str {
        "raw \u{1b}[31mred\u{1b}[0m \u{1b}]0;title\u{7} \
         before\rrewritten\u{8}x\0 bidi \u{202e}spoof\u{202c}"
    }

    fn assert_no_cli_display_markers(value: &str) {
        for marker in [
            "<ESC>", "<BEL>", "<CR>", "<BS>", "<NUL>", "<U+202E>", "<U+202C>",
        ] {
            assert!(!value.contains(marker), "unexpected marker {marker}");
        }
    }

    #[test]
    fn chat_completion_response_preserves_raw_assistant_text_as_api_data() {
        let assistant_text = format!(
            "# Heading\n\n\
            | key | value |\n\
            | --- | --- |\n\
            | path | C:\\Users\\rdime\\Desktop\\gpt-cli |\n\n\
            stderr: error: assistant text\n\
            {}",
            raw_assistant_text()
        );

        let response = build_chat_completion_response(
            "req-1",
            "job-1",
            "tablinq-instant",
            RequestRole::Primary,
            None,
            Some("conv-1"),
            Some("browser-primary"),
            &assistant_text,
            "stop",
            None,
        );
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        let json_text = String::from_utf8(response.body).unwrap();

        assert_eq!(
            body["choices"][0]["message"]["content"].as_str(),
            Some(assistant_text.as_str())
        );
        assert_no_cli_display_markers(&json_text);
    }
}
