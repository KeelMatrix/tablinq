use crate::chat::PreparedChatRequest;
use crate::http::RequestContext;
use crate::openai::request_role_name;
use gptd::CoreService;
use gptd::core::error_code_for_message;
use shared_protocol::{ErrorCode, GatewayRequestContext};

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

pub(crate) fn capture_gateway_failure_artifacts(
    core: &CoreService,
    context: &RequestContext,
    prepared: &PreparedChatRequest,
    client_id: &str,
    err: &anyhow::Error,
) -> Option<String> {
    let include_request_evidence = should_capture_request_evidence(core, Some(err), false);
    let include_failure_bundle = should_capture_gateway_failure_bundle(err);
    if !include_request_evidence && !include_failure_bundle {
        return None;
    }

    let message = format!("{err:#}");
    let gateway_request = include_failure_bundle.then(|| GatewayRequestContext {
        request_id: context.request_id.clone(),
        request_path: context.request.path.clone(),
        client_id: Some(client_id.to_owned()),
        model: Some(prepared.model_id.to_owned()),
        request_role: Some(request_role_name(prepared.request_role).to_owned()),
        parent_request_id: prepared.parent_request_id.clone(),
        job_id: Some(prepared.job_id().to_owned()),
        streaming: prepared.request.stream,
    });

    core.capture_gateway_failure_artifacts(
        &context.request_id,
        Some(prepared.mode()),
        &message,
        gateway_request,
        include_request_evidence,
    )
    .ok()
    .flatten()
}
