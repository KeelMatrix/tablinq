use crate::http::HttpResponse;
use crate::json::json_response;
use crate::openai::openai_error;
use gptd::browser::execution_error_metadata;
use serde_json::{Value, json};

use super::classification::map_openai_error_parts;
use super::guidance::{provider_guidance_for_code, refusal_reason_for_code};

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
    if let Some(diagnostic) =
        execution_metadata.and_then(|item| item.native_upload_rate_limit.as_ref())
    {
        value["tablinq_native_upload_budget"] =
            serde_json::to_value(diagnostic).unwrap_or_else(|_| json!({}));
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

pub(crate) fn recovery_strategy_name(strategy: &shared_protocol::RecoveryStrategy) -> &'static str {
    match strategy {
        shared_protocol::RecoveryStrategy::ReloadNewConversationReplayWithContext => {
            "reload_new_conversation_replay_with_context"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;
    use gptd::browser::{ExecutionErrorMetadata, execution_error};
    use serde_json::Value;
    use shared_protocol::{
        Effort, ErrorCode, Mode, ModelClass, RecoveryPacket, RecoveryStrategy, RecoveryTrace,
    };

    fn assert_no_cli_display_markers(value: &str) {
        for marker in [
            "<ESC>", "<BEL>", "<CR>", "<BS>", "<NUL>", "<U+202E>", "<U+202C>",
        ] {
            assert!(!value.contains(marker), "unexpected marker {marker}");
        }
    }

    #[test]
    fn openai_error_mapping_preserves_refusal_annotations() {
        let err =
            anyhow!("auth required: attached browser-backed ChatGPT tab is not authenticated");
        let response = map_openai_error("req-1", &err, Some("bundle-1"));
        assert_eq!(response.status_code, 503);

        let body: Value = serde_json::from_slice(&response.body).expect("decode error body");
        assert_eq!(body["request_id"], "req-1");
        assert_eq!(body["error"]["code"], "auth_required");
        assert_eq!(body["error"]["param"], Value::Null);
        assert_eq!(
            body["error"]["message"],
            "req-1: auth required: attached browser-backed ChatGPT tab is not authenticated"
        );
        assert_eq!(body["tablinq_refusal_reason"], "auth_required");
        assert_eq!(body["tablinq_failure_bundle_id"], "bundle-1");
        assert!(
            body["tablinq_guidance"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
        assert_no_cli_display_markers(&String::from_utf8(response.body).unwrap());
    }

    #[test]
    fn provider_generation_errors_include_recovery_metadata() {
        let recovery_trace = RecoveryTrace {
            recovery_available: true,
            recovery_attempted: true,
            recovery_strategy: Some(RecoveryStrategy::ReloadNewConversationReplayWithContext),
            failure_code: Some(ErrorCode::ProviderGenerationFailedNonretryable),
            failure_bundle_id: Some("bundle-provider-1".to_owned()),
            prior_request_id: Some("req-prior".to_owned()),
            replacement_request_id: Some("req-replacement".to_owned()),
            prior_conversation_id: Some("conv-prior".to_owned()),
            replacement_conversation_id: Some("conv-replacement".to_owned()),
            recovery_attempt_count: 1,
        };
        let recovery_packet = RecoveryPacket {
            original_input: "retry the prompt".to_owned(),
            mode: Mode::Normal,
            model_class: Some(ModelClass::Thinking),
            effort: Some(Effort::Standard),
            files: Vec::new(),
            context_summary: None,
            context_manifest: None,
            recovery_manifest_path: None,
            replay_files: Vec::new(),
            prior_request_id: Some("req-prior".to_owned()),
            prior_conversation_id: Some("conv-prior".to_owned()),
            failure_code: ErrorCode::ProviderGenerationFailedNonretryable,
            recovery_attempt_count: 1,
        };
        let err = execution_error(
            ExecutionErrorMetadata::new(
                ErrorCode::ProviderGenerationFailedNonretryable,
                "provider generation failed",
                Some("https://chatgpt.test/c/conv-prior".to_owned()),
            )
            .with_recovery(recovery_trace, recovery_packet),
        );

        let body = map_openai_error_value("req-provider", &err, None);

        assert_eq!(
            body["error"]["code"],
            "provider_generation_failed_nonretryable"
        );
        assert_eq!(body["error"]["param"], Value::Null);
        assert_eq!(body["status"], 502);
        assert_eq!(body["request_id"], "req-provider");
        assert_eq!(body["tablinq_failure_bundle_id"], "bundle-provider-1");
        assert_eq!(body["tablinq_recovery_available"], true);
        assert_eq!(body["tablinq_recovery_attempted"], true);
        assert_eq!(
            body["tablinq_recovery_strategy"],
            "reload_new_conversation_replay_with_context"
        );
        assert_eq!(body["tablinq_prior_request_id"], "req-prior");
        assert_eq!(body["tablinq_replacement_request_id"], "req-replacement");
        assert_eq!(
            body["tablinq_replacement_conversation_id"],
            "conv-replacement"
        );
    }
}
