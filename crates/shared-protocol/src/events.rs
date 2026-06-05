use crate::validation::{
    ValidationError, validate_exact_version, validate_optional, validate_required,
};
use crate::{ArtifactMetadata, CapabilityMatrix, CapabilityReport, JobStatus, Mode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Transport,
    BrowserLaunch,
    AttachEndpointUnavailable,
    ProfileLocked,
    AuthRequired,
    CapabilityMissing,
    ActionTimeout,
    UiDrift,
    DownloadFailed,
    UploadFailed,
    ResearchJobFailed,
    ProviderGenerationFailedNonretryable,
    ProviderStoppedThinking,
    ProviderConversationLoadFailed,
    NativeUploadRateLimited,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StreamChunkKind {
    Delta,
    Citation,
    Status,
    Done,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    Ok,
    Degraded,
    Down,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EventEnvelope {
    pub version: String,
    pub request_id: Option<String>,
    pub emitted_at: DateTime<Utc>,
    pub event: EventPayload,
}

impl EventEnvelope {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_exact_version(&self.version)?;
        validate_optional("request_id", self.request_id.as_deref())?;
        self.event.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum EventPayload {
    Ack(AckEvent),
    StreamChunk(StreamChunk),
    CapabilityReport(CapabilityReport),
    JobState(JobState),
    ArtifactReady(ArtifactMetadata),
    Failure(FailureBundle),
    HealthCheck(HealthCheck),
}

impl EventPayload {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Ack(event) => event.validate(),
            Self::StreamChunk(event) => event.validate(),
            Self::CapabilityReport(event) => event.validate(),
            Self::JobState(event) => event.validate(),
            Self::ArtifactReady(event) => event.validate(),
            Self::Failure(event) => event.validate(),
            Self::HealthCheck(event) => event.validate(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AckEvent {
    pub accepted: bool,
    pub message: String,
}

impl AckEvent {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("message", &self.message)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StreamChunk {
    pub job_id: String,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub seq: u64,
    pub kind: StreamChunkKind,
    pub text: String,
}

impl StreamChunk {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("job_id", &self.job_id)?;
        validate_optional("conversation_id", self.conversation_id.as_deref())?;
        validate_optional("message_id", self.message_id.as_deref())?;
        if self.kind != StreamChunkKind::Done {
            validate_required("text", &self.text)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JobState {
    pub job_id: String,
    pub status: JobStatus,
    pub message: Option<String>,
    pub conversation_id: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub artifacts: Vec<ArtifactMetadata>,
}

impl JobState {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("job_id", &self.job_id)?;
        validate_optional("message", self.message.as_deref())?;
        validate_optional("conversation_id", self.conversation_id.as_deref())?;
        for artifact in &self.artifacts {
            artifact.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FailureBundle {
    pub bundle_id: String,
    pub error_code: ErrorCode,
    pub message: String,
    pub created_at: DateTime<Utc>,
    pub url: Option<String>,
    pub mode: Option<Mode>,
    pub screenshot_path: Option<String>,
    pub dom_snapshot_path: Option<String>,
    pub aria_snapshot_path: Option<String>,
    pub logs_path: Option<String>,
    pub strategy_attempts: Vec<StrategyAttempt>,
    pub capability_matrix: CapabilityMatrix,
    pub gateway_request: Option<GatewayRequestContext>,
}

impl FailureBundle {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("bundle_id", &self.bundle_id)?;
        validate_required("message", &self.message)?;
        validate_optional("url", self.url.as_deref())?;
        validate_optional("screenshot_path", self.screenshot_path.as_deref())?;
        validate_optional("dom_snapshot_path", self.dom_snapshot_path.as_deref())?;
        validate_optional("aria_snapshot_path", self.aria_snapshot_path.as_deref())?;
        validate_optional("logs_path", self.logs_path.as_deref())?;
        for attempt in &self.strategy_attempts {
            attempt.validate()?;
        }
        if let Some(context) = &self.gateway_request {
            context.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GatewayRequestContext {
    pub request_id: String,
    pub request_path: String,
    pub client_id: Option<String>,
    pub model: Option<String>,
    pub request_role: Option<String>,
    pub parent_request_id: Option<String>,
    pub job_id: Option<String>,
    pub streaming: bool,
}

impl GatewayRequestContext {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("request_id", &self.request_id)?;
        validate_required("request_path", &self.request_path)?;
        validate_optional("client_id", self.client_id.as_deref())?;
        validate_optional("model", self.model.as_deref())?;
        validate_optional("request_role", self.request_role.as_deref())?;
        validate_optional("parent_request_id", self.parent_request_id.as_deref())?;
        validate_optional("job_id", self.job_id.as_deref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StrategyAttempt {
    pub capability: String,
    pub strategy_id: String,
    pub score: f32,
    pub outcome: String,
    pub detail: Option<String>,
}

impl StrategyAttempt {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("capability", &self.capability)?;
        validate_required("strategy_id", &self.strategy_id)?;
        validate_required("outcome", &self.outcome)?;
        validate_optional("detail", self.detail.as_deref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HealthCheck {
    pub status: HealthStatus,
    pub checked_at: DateTime<Utc>,
    pub browser_connected: bool,
    pub extension_connected: bool,
    pub authenticated: bool,
    pub details: Option<String>,
}

impl HealthCheck {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_optional("details", self.details.as_deref())
    }
}
