use gptd::browser::execution_error_metadata;
use gptd::core::error_code_for_message;
use shared_protocol::ErrorCode;

pub(crate) fn map_openai_error_parts(
    err: &anyhow::Error,
) -> (u16, &'static str, &'static str, String) {
    if let Some(metadata) = execution_error_metadata(err)
        && matches!(
            metadata.error_code,
            ErrorCode::ProviderGenerationFailedNonretryable
                | ErrorCode::ProviderConversationLoadFailed
        )
    {
        let code = match metadata.error_code {
            ErrorCode::ProviderGenerationFailedNonretryable => {
                "provider_generation_failed_nonretryable"
            }
            ErrorCode::ProviderConversationLoadFailed => "provider_conversation_load_failed",
            _ => unreachable!(),
        };
        return (502, "provider_error", code, metadata.message);
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
        ErrorCode::BrowserLaunch
        | ErrorCode::AttachEndpointUnavailable
        | ErrorCode::ProfileLocked => (
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
        ErrorCode::ProviderConversationLoadFailed => (
            502,
            "provider_error",
            "provider_conversation_load_failed",
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
