use crate::chat::PreparedChatRequest;
use crate::http::{HttpResponse, RequestContext};
use crate::json::json_response;
use crate::openai::{RequestRole, openai_error, request_role_name};
use gptd::CoreService;
use gptd::browser::execution_error_metadata;
use gptd::core::error_code_for_message;
use serde_json::{Value, json};
use shared_protocol::{ErrorCode, GatewayRequestContext};

pub(crate) fn request_role_policy_response(
    request_id: &str,
    role: RequestRole,
) -> Option<HttpResponse> {
    match role {
        RequestRole::Primary | RequestRole::Branch => None,
        RequestRole::Subagent => {
            let mut body = openai_error(
                429,
                "overloaded_error",
                "subagents_disabled",
                &format!(
                    "{request_id}: subagent requests are disabled by default for the single-browser OpenClaude preset"
                ),
            );
            body["request_id"] = Value::String(request_id.to_owned());
            body["tablinq_refusal_reason"] = Value::String("subagents_disabled".to_owned());
            body["tablinq_guidance"] = json!(provider_guidance_for_code("subagents_disabled"));
            Some(json_response(429, body))
        }
    }
}

pub(crate) fn capture_gateway_failure_bundle(
    core: &CoreService,
    context: &RequestContext,
    prepared: &PreparedChatRequest,
    client_id: &str,
    err: &anyhow::Error,
) -> Option<String> {
    if !should_capture_gateway_failure_bundle(err) {
        return None;
    }
    let message = format!("{err:#}");
    core.capture_failure_bundle(
        Some(prepared.mode()),
        &message,
        Some(GatewayRequestContext {
            request_id: context.request_id.clone(),
            request_path: context.request.path.clone(),
            client_id: Some(client_id.to_owned()),
            model: Some(prepared.model_id.to_owned()),
            request_role: Some(request_role_name(prepared.request_role).to_owned()),
            parent_request_id: prepared.parent_request_id.clone(),
            job_id: Some(prepared.job_id().to_owned()),
            streaming: prepared.request.stream,
        }),
    )
    .ok()
    .map(|bundle| bundle.bundle_id)
}

pub(crate) fn should_capture_gateway_failure_bundle(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}");
    matches!(
        error_code_for_message(&message),
        ErrorCode::AuthRequired
            | ErrorCode::CapabilityMissing
            | ErrorCode::BrowserLaunch
            | ErrorCode::ProfileLocked
            | ErrorCode::ActionTimeout
            | ErrorCode::UiDrift
            | ErrorCode::ResearchJobFailed
    )
}

pub(crate) fn provider_guidance_for_code(code: &str) -> Vec<&'static str> {
    match code {
        "auth_required" => vec![
            "provider mode refuses requests when the managed browser is signed out; sign in in the managed profile and retry",
            "run gpt doctor or GET /ready to confirm the browser session is authenticated before retrying queued work",
        ],
        "capability_missing" => vec![
            "provider mode refuses requests when the selected model or mode is unavailable instead of guessing a fallback",
            "refresh GET /v1/capabilities after account, workspace, or UI changes before retrying the request",
        ],
        "browser_unavailable" => vec![
            "provider mode refuses requests until the managed browser can be opened exclusively",
            "close other managed-profile browser instances or reset the managed profile if the browser stays unavailable",
        ],
        "upstream_timeout" => vec![
            "provider mode refuses long-stalled requests instead of inventing a partial answer",
            "inspect the latest failure bundle and retry after the managed tab is responsive again",
        ],
        "ui_drift" => vec![
            "provider mode refuses when the live page no longer matches known controls",
            "inspect the linked failure bundle before retrying so selector or capability drift is actionable",
        ],
        "research_job_failed" => vec![
            "provider mode refuses failed research runs instead of returning incomplete research output",
        ],
        "subagents_disabled" => vec![
            "provider mode keeps subagents disabled by default to protect the single warm browser session",
        ],
        _ => Vec::new(),
    }
}

pub(crate) fn refusal_reason_for_code(code: &str) -> Option<&str> {
    match code {
        "auth_required"
        | "capability_missing"
        | "browser_unavailable"
        | "upstream_timeout"
        | "ui_drift"
        | "research_job_failed"
        | "subagents_disabled" => Some(code),
        _ => None,
    }
}

pub(crate) fn maybe_capture_request_evidence(
    core: &CoreService,
    request_id: &str,
    err: Option<&anyhow::Error>,
    low_confidence: bool,
) -> Option<String> {
    if !should_capture_request_evidence(core, err, low_confidence) {
        return None;
    }
    core.capture_browser_evidence(request_id)
        .ok()
        .map(|artifact| artifact.artifact_id)
}

pub(crate) fn should_capture_request_evidence(
    core: &CoreService,
    err: Option<&anyhow::Error>,
    low_confidence: bool,
) -> bool {
    if low_confidence {
        return true;
    }

    let message = err.map(|value| format!("{value:#}")).unwrap_or_default();
    if message.contains("timed out")
        || message.contains("reload")
        || message.contains("extension")
        || message.contains("disconnect")
        || message.contains("reconcil")
    {
        return true;
    }

    core.browser_profile_info()
        .is_ok_and(|snapshot| !snapshot.extension_connected || snapshot.reload_count >= 2)
}

pub(crate) fn map_openai_error(
    request_id: &str,
    err: &anyhow::Error,
    failure_bundle_id: Option<&str>,
) -> HttpResponse {
    let (status, error_type, code, message) = map_openai_error_parts(err);
    let execution_metadata = execution_error_metadata(err);
    json_response(
        status,
        annotate_openai_error_value(
            request_id,
            openai_error(
                status,
                error_type,
                code,
                &format!("{request_id}: {message}"),
            ),
            code,
            failure_bundle_id,
            execution_metadata.as_ref(),
        ),
    )
}

pub(crate) fn map_openai_error_value(
    request_id: &str,
    err: &anyhow::Error,
    failure_bundle_id: Option<&str>,
) -> Value {
    let (status, error_type, code, message) = map_openai_error_parts(err);
    let execution_metadata = execution_error_metadata(err);
    annotate_openai_error_value(
        request_id,
        openai_error(status, error_type, code, &message),
        code,
        failure_bundle_id,
        execution_metadata.as_ref(),
    )
}

pub(crate) fn annotate_openai_error_value(
    request_id: &str,
    mut value: Value,
    code: &str,
    failure_bundle_id: Option<&str>,
    execution_metadata: Option<&gptd::browser::ExecutionErrorMetadata>,
) -> Value {
    value["request_id"] = Value::String(request_id.to_owned());
    let recovery_trace = execution_metadata.and_then(|item| item.recovery_trace.as_ref());
    if let Some(bundle_id) = recovery_trace
        .and_then(|trace| trace.failure_bundle_id.as_deref())
        .or(failure_bundle_id)
    {
        value["tablinq_failure_bundle_id"] = Value::String(bundle_id.to_owned());
    }
    value["tablinq_recovery_available"] = Value::Bool(
        recovery_trace
            .map(|trace| trace.recovery_available)
            .unwrap_or(false),
    );
    value["tablinq_recovery_attempted"] = Value::Bool(
        recovery_trace
            .map(|trace| trace.recovery_attempted)
            .unwrap_or(false),
    );
    if let Some(strategy) = recovery_trace
        .and_then(|trace| trace.recovery_strategy.as_ref())
        .map(recovery_strategy_name)
    {
        value["tablinq_recovery_strategy"] = Value::String(strategy.to_owned());
    }
    if let Some(prior_request_id) =
        recovery_trace.and_then(|trace| trace.prior_request_id.as_deref())
    {
        value["tablinq_prior_request_id"] = Value::String(prior_request_id.to_owned());
    }
    if let Some(replacement_request_id) =
        recovery_trace.and_then(|trace| trace.replacement_request_id.as_deref())
    {
        value["tablinq_replacement_request_id"] = Value::String(replacement_request_id.to_owned());
    }
    if let Some(replacement_conversation_id) =
        recovery_trace.and_then(|trace| trace.replacement_conversation_id.as_deref())
    {
        value["tablinq_replacement_conversation_id"] =
            Value::String(replacement_conversation_id.to_owned());
    }
    let guidance = provider_guidance_for_code(code);
    if !guidance.is_empty() {
        value["tablinq_guidance"] = json!(guidance);
    }
    if let Some(reason) = refusal_reason_for_code(code) {
        value["tablinq_refusal_reason"] = Value::String(reason.to_owned());
    }
    value
}

pub(crate) fn map_openai_error_parts(
    err: &anyhow::Error,
) -> (u16, &'static str, &'static str, String) {
    if let Some(metadata) = execution_error_metadata(err)
        && matches!(
            metadata.error_code,
            ErrorCode::ProviderGenerationFailedNonretryable
        )
    {
        return (
            502,
            "provider_error",
            "provider_generation_failed_nonretryable",
            metadata.message,
        );
    }

    let message = format!("{err:#}");
    if message.contains("cancelled") {
        return (409, "invalid_request_error", "cancelled", message);
    }
    if message.contains("was not found") && message.contains("conversation") {
        return (
            404,
            "invalid_request_error",
            "conversation_not_found",
            message,
        );
    }
    if message.contains("staged file") {
        return (400, "invalid_request_error", "file_not_found", message);
    }
    match error_code_for_message(&message) {
        ErrorCode::CapabilityMissing => {
            (400, "invalid_request_error", "capability_missing", message)
        }
        ErrorCode::AuthRequired => (503, "authentication_error", "auth_required", message),
        ErrorCode::BrowserLaunch | ErrorCode::ProfileLocked => (
            503,
            "service_unavailable_error",
            "browser_unavailable",
            message,
        ),
        ErrorCode::ActionTimeout => (504, "timeout_error", "upstream_timeout", message),
        ErrorCode::ProviderGenerationFailedNonretryable => (
            502,
            "provider_error",
            "provider_generation_failed_nonretryable",
            message,
        ),
        ErrorCode::UiDrift => (502, "provider_error", "ui_drift", message),
        ErrorCode::ResearchJobFailed => (502, "provider_error", "research_job_failed", message),
        ErrorCode::UploadFailed => (400, "invalid_request_error", "upload_failed", message),
        ErrorCode::DownloadFailed => (502, "provider_error", "download_failed", message),
        ErrorCode::Transport => {
            if message.contains("queue full") {
                (429, "overloaded_error", "queue_full", message)
            } else if message.contains("queue timeout") {
                (429, "overloaded_error", "queue_timeout", message)
            } else if message.contains("per-client queue limit") {
                (429, "overloaded_error", "client_limit_exceeded", message)
            } else {
                (500, "server_error", "transport_error", message)
            }
        }
    }
}

pub(crate) fn recovery_strategy_name(strategy: &shared_protocol::RecoveryStrategy) -> &'static str {
    match strategy {
        shared_protocol::RecoveryStrategy::ReloadNewConversationReplayWithContext => {
            "reload_new_conversation_replay_with_context"
        }
    }
}

pub(crate) fn is_client_disconnect_error(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}").to_ascii_lowercase();
    message.contains("broken pipe")
        || message.contains("connection reset")
        || message.contains("connection abort")
        || message.contains("forcibly closed")
        || message.contains("os error 10053")
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn openai_error_mapping_preserves_refusal_annotations() {
        let err = anyhow!("auth required: managed browser session is not authenticated");
        let response = map_openai_error("req-1", &err, Some("bundle-1"));
        assert_eq!(response.status_code, 503);

        let body: Value = serde_json::from_slice(&response.body).expect("decode error body");
        assert_eq!(body["request_id"], "req-1");
        assert_eq!(body["error"]["code"], "auth_required");
        assert_eq!(body["tablinq_refusal_reason"], "auth_required");
        assert_eq!(body["tablinq_failure_bundle_id"], "bundle-1");
        assert!(
            body["tablinq_guidance"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
    }

    #[test]
    fn client_disconnect_detection_covers_common_socket_errors() {
        assert!(is_client_disconnect_error(&anyhow!(
            "broken pipe while streaming"
        )));
        assert!(is_client_disconnect_error(&anyhow!(
            "connection reset by peer"
        )));
        assert!(!is_client_disconnect_error(&anyhow!("provider timed out")));
    }
}
