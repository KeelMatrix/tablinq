use anyhow::Error;
use shared_protocol::{ErrorCode, RecoveryPacket, RecoveryTrace};
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone)]
pub struct ExecutionErrorMetadata {
    pub error_code: ErrorCode,
    pub message: String,
    pub current_url: Option<String>,
    pub recovery_trace: Option<RecoveryTrace>,
    pub recovery_packet: Option<RecoveryPacket>,
}

#[derive(Debug, Clone)]
struct TypedExecutionError(ExecutionErrorMetadata);

impl ExecutionErrorMetadata {
    pub fn new(
        error_code: ErrorCode,
        message: impl Into<String>,
        current_url: Option<String>,
    ) -> Self {
        Self {
            error_code,
            message: message.into(),
            current_url,
            recovery_trace: None,
            recovery_packet: None,
        }
    }

    pub fn with_recovery(
        mut self,
        recovery_trace: RecoveryTrace,
        recovery_packet: RecoveryPacket,
    ) -> Self {
        self.recovery_trace = Some(recovery_trace);
        self.recovery_packet = Some(recovery_packet);
        self
    }
}

impl Display for TypedExecutionError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0.message)
    }
}

impl std::error::Error for TypedExecutionError {}

pub fn adapter_error_code(value: &str) -> Option<ErrorCode> {
    match value {
        "provider_generation_failed_nonretryable" => {
            Some(ErrorCode::ProviderGenerationFailedNonretryable)
        }
        "provider_stopped_thinking" => Some(ErrorCode::ProviderStoppedThinking),
        "provider_conversation_load_failed" => Some(ErrorCode::ProviderConversationLoadFailed),
        _ => None,
    }
}

pub fn execution_error(metadata: ExecutionErrorMetadata) -> Error {
    Error::new(TypedExecutionError(metadata))
}

pub fn execution_error_metadata(err: &Error) -> Option<ExecutionErrorMetadata> {
    err.downcast_ref::<TypedExecutionError>()
        .map(|error| error.0.clone())
}
