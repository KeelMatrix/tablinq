use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const PROTOCOL_VERSION: &str = "tablinq.v1";
pub const SELECTOR_PACK_FORMAT_VERSION: u32 = 1;
pub const CONFIG_VERSION: u32 = 2;
pub const APP_NAME: &str = "Tablinq";
pub const CHROME_ATTACH_PROFILE_DIR_NAME: &str = "chrome-attach-profile";
pub const CHROME_ATTACH_PROFILE_NOTE: &str = "Chrome attach-first uses a dedicated persistent real profile under the Tablinq state root. Sign in once in that profile before attaching.";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildMetadata {
    pub application: String,
    pub version: String,
    pub git_commit: Option<String>,
    pub git_dirty: bool,
    pub target: String,
    pub profile: String,
    pub built_at_unix_epoch: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Normal,
    Search,
    Research,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ModelClass {
    Instant,
    Thinking,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    None,
    Standard,
    Extended,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Transport,
    BrowserLaunch,
    ProfileLocked,
    AuthRequired,
    CapabilityMissing,
    ActionTimeout,
    UiDrift,
    DownloadFailed,
    UploadFailed,
    ResearchJobFailed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Queued,
    Starting,
    Running,
    Interrupted,
    Recovered,
    WaitingForSources,
    Completed,
    Cancelled,
    Failed,
    ArtifactReady,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Ask,
    Research,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestExecutionKind {
    Ask,
    Research,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrowserConnectionMode {
    #[default]
    AttachFirst,
    ManagedFallback,
    ManagedOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrowserOwner {
    External,
    #[default]
    Managed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthSource {
    Page,
    Extension,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    AriaRole,
    Css,
    Text,
    DomNeighborhood,
    Hotkey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandEnvelope {
    pub version: String,
    pub request_id: String,
    pub session_id: Option<String>,
    pub sent_at: DateTime<Utc>,
    pub command: CommandPayload,
}

impl CommandEnvelope {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_exact_version(&self.version)?;
        validate_required("request_id", &self.request_id)?;
        validate_optional("session_id", self.session_id.as_deref())?;
        self.command.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum CommandPayload {
    Ask(AskCommand),
    Research(ResearchCommand),
    CancelJob(CancelJobCommand),
    InspectCapabilities(InspectCapabilitiesCommand),
    HealthPing(HealthPingCommand),
    PullArtifact(PullArtifactCommand),
}

impl CommandPayload {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Ask(command) => command.validate(),
            Self::Research(command) => command.validate(),
            Self::CancelJob(command) => command.validate(),
            Self::InspectCapabilities(command) => command.validate(),
            Self::HealthPing(command) => command.validate(),
            Self::PullArtifact(command) => command.validate(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AskCommand {
    pub prompt: String,
    pub conversation_id: Option<String>,
    pub new_conversation: bool,
    pub mode: Mode,
    pub model_class: Option<ModelClass>,
    pub effort: Option<Effort>,
    pub files: Vec<String>,
}

impl AskCommand {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("prompt", &self.prompt)?;
        validate_optional("conversation_id", self.conversation_id.as_deref())?;
        if self.new_conversation && self.conversation_id.is_some() {
            return Err(ValidationError::InvalidFieldCombination {
                field: "conversation_id".to_owned(),
                message: "conversation_id cannot be combined with new_conversation".to_owned(),
            });
        }
        validate_paths("files", &self.files)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResearchCommand {
    pub query: String,
    pub conversation_id: Option<String>,
    pub new_conversation: bool,
    pub files: Vec<String>,
    pub output_dir: Option<String>,
}

impl ResearchCommand {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("query", &self.query)?;
        validate_optional("conversation_id", self.conversation_id.as_deref())?;
        if self.new_conversation && self.conversation_id.is_some() {
            return Err(ValidationError::InvalidFieldCombination {
                field: "conversation_id".to_owned(),
                message: "conversation_id cannot be combined with new_conversation".to_owned(),
            });
        }
        validate_optional("output_dir", self.output_dir.as_deref())?;
        validate_paths("files", &self.files)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CancelJobCommand {
    pub job_id: String,
}

impl CancelJobCommand {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("job_id", &self.job_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct InspectCapabilitiesCommand {}

impl InspectCapabilitiesCommand {
    pub fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct HealthPingCommand {}

impl HealthPingCommand {
    pub fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullArtifactCommand {
    pub artifact_id: String,
    pub destination_dir: String,
}

impl PullArtifactCommand {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("artifact_id", &self.artifact_id)?;
        validate_required("destination_dir", &self.destination_dir)
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct CapabilityMatrix {
    pub composer_ready: bool,
    pub submit_available: bool,
    pub model_picker_visible: bool,
    pub thinking_effort_visible: bool,
    pub search_mode_visible: bool,
    pub research_mode_visible: bool,
    pub upload_control_visible: bool,
    pub downloadable_artifacts_observable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CapabilityReport {
    pub reported_at: DateTime<Utc>,
    pub selector_pack_id: String,
    pub selector_pack_version: String,
    pub matrix: CapabilityMatrix,
    pub aliases: RuntimeAliases,
}

impl CapabilityReport {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("selector_pack_id", &self.selector_pack_id)?;
        validate_required("selector_pack_version", &self.selector_pack_version)?;
        self.aliases.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct RuntimeAliases {
    pub modes: BTreeMap<String, Vec<String>>,
    pub model_classes: BTreeMap<String, Vec<String>>,
    pub efforts: BTreeMap<String, Vec<String>>,
}

impl RuntimeAliases {
    pub fn validate(&self) -> Result<(), ValidationError> {
        for (field, groups) in [
            ("aliases.modes", &self.modes),
            ("aliases.model_classes", &self.model_classes),
            ("aliases.efforts", &self.efforts),
        ] {
            for (name, labels) in groups {
                validate_required(field, name)?;
                for label in labels {
                    validate_required(field, label)?;
                }
            }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactMetadata {
    pub artifact_id: String,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub file_name: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub storage_path: String,
}

impl ArtifactMetadata {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("artifact_id", &self.artifact_id)?;
        validate_optional("conversation_id", self.conversation_id.as_deref())?;
        validate_optional("message_id", self.message_id.as_deref())?;
        validate_required("file_name", &self.file_name)?;
        validate_required("content_type", &self.content_type)?;
        validate_required("sha256", &self.sha256)?;
        validate_required("storage_path", &self.storage_path)
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SelectorPack {
    pub format_version: u32,
    pub pack_id: String,
    pub description: String,
    pub updated_at: DateTime<Utc>,
    pub capabilities: BTreeMap<String, Vec<SelectorStrategy>>,
}

impl SelectorPack {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.format_version != SELECTOR_PACK_FORMAT_VERSION {
            return Err(ValidationError::UnsupportedSelectorPackFormat {
                found: self.format_version,
                expected: SELECTOR_PACK_FORMAT_VERSION,
            });
        }

        validate_required("pack_id", &self.pack_id)?;
        validate_required("description", &self.description)?;

        if self.capabilities.is_empty() {
            return Err(ValidationError::EmptyCollection("capabilities"));
        }

        for (capability, strategies) in &self.capabilities {
            validate_required("capabilities", capability)?;
            if strategies.is_empty() {
                return Err(ValidationError::EmptyCollection("capability strategies"));
            }

            let mut seen = BTreeSet::new();
            for strategy in strategies {
                strategy.validate()?;
                if !seen.insert(strategy.strategy_id.clone()) {
                    return Err(ValidationError::DuplicateStrategy {
                        field: capability.clone(),
                        strategy_id: strategy.strategy_id.clone(),
                    });
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SelectorStrategy {
    pub strategy_id: String,
    pub kind: StrategyKind,
    pub selector: String,
    pub score: f32,
}

impl SelectorStrategy {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("strategy_id", &self.strategy_id)?;
        validate_required("selector", &self.selector)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum DaemonRequest {
    Ping,
    Status,
    Shutdown,
    Doctor,
    Ask(AskCommand),
    Research(ResearchCommand),
    Capabilities,
    ConfigShow,
    SessionsList,
    JobsList,
    JobsShow(String),
    JobsCancel(CancelJobCommand),
    ArtifactsList,
    ArtifactPull(PullArtifactCommand),
    BrowserOpen(BrowserOpenRequest),
    BrowserClose,
    BrowserLogout,
    BrowserResetProfile,
    BrowserProfileInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum DaemonResponse {
    Ack(AckEvent),
    Health(HealthCheck),
    Status(DaemonStatusSnapshot),
    Doctor(DoctorReport),
    Ask(AskResult),
    Research(ResearchResult),
    Capabilities(CapabilitySnapshot),
    Config(ConfigSnapshot),
    Sessions(SessionList),
    Jobs(JobList),
    Job(JobDetail),
    Artifacts(ArtifactList),
    ArtifactPull(ArtifactPullResult),
    Browser(BrowserStatusSnapshot),
    Error(DaemonErrorResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum DaemonFrame {
    Event(EventPayload),
    Response(DaemonResponse),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DaemonStatusSnapshot {
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub state_root: String,
    pub ipc_endpoint: String,
    pub lock_path: String,
    pub db_path: String,
    pub warm_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfigSnapshot {
    pub host: String,
    pub port: u16,
    pub api_key_present: bool,
    pub max_concurrent_upstream_jobs: u32,
    pub queue_length: u32,
    pub queue_timeout_ms: u64,
    pub per_client_limit: u32,
    pub request_timeout_ms: u64,
    pub ask_timeout_ms: u64,
    pub research_timeout_ms: u64,
    pub safe_mode_behavior: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConfigSnapshot {
    pub version: u32,
    pub state_root: String,
    pub config_dir: String,
    pub config_path: String,
    pub logs_dir: String,
    pub log_path: String,
    pub db_dir: String,
    pub db_path: String,
    pub browser_profile_dir: String,
    pub chrome_attach_profile_dir: String,
    pub downloads_dir: String,
    pub crash_bundles_dir: String,
    pub ipc_endpoint: String,
    pub warm_mode: bool,
    pub safe_mode: bool,
    pub browser_target_url: String,
    pub browser_path: Option<String>,
    pub browser_connection_mode: BrowserConnectionMode,
    pub browser_debug_endpoint: String,
    pub browser_debug_host: String,
    pub browser_debug_port: u16,
    pub browser_channel: String,
    pub browser_extension_required: bool,
    pub browser_auto_open_target_tab: bool,
    pub gateway: GatewayConfigSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SelectorPackSummary {
    pub pack_id: String,
    pub version: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StrategyTelemetryEntry {
    pub capability: String,
    pub strategy_id: String,
    pub success_count: u64,
    pub failure_count: u64,
    pub success_rate: f32,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BrowserDoctor {
    pub browser_detected: bool,
    pub detected_browser_path: Option<String>,
    pub profile_accessible: bool,
    pub profile_access_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DoctorReport {
    pub health: HealthCheck,
    pub status: DaemonStatusSnapshot,
    pub config: ConfigSnapshot,
    pub browser: BrowserStatusSnapshot,
    pub capabilities: Option<CapabilityReport>,
    pub selector_pack: Option<SelectorPackSummary>,
    pub strategy_telemetry: Vec<StrategyTelemetryEntry>,
    pub failure_bundle_count: u64,
    pub latest_failure_bundle: Option<FailureBundle>,
    pub browser_detection: BrowserDoctor,
    pub recovery_guidance: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySnapshot {
    pub report: Option<CapabilityReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionSummary {
    pub session_id: String,
    pub label: String,
    pub tab_url: Option<String>,
    pub last_seen_at: DateTime<Utc>,
    pub latest_conversation_id: Option<String>,
    pub recent_conversations: Vec<ConversationSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionList {
    pub sessions: Vec<SessionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSummary {
    pub conversation_id: String,
    pub session_id: String,
    pub label: String,
    pub tab_url: Option<String>,
    pub last_prompt: Option<String>,
    pub last_response: Option<String>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JobSummary {
    pub job_id: String,
    pub status: JobStatus,
    pub message: Option<String>,
    pub conversation_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JobList {
    pub jobs: Vec<JobSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequestUsage {
    pub status: String,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequestSummary {
    pub request_id: String,
    pub job_id: String,
    pub model_id: Option<String>,
    pub request_role: Option<String>,
    pub parent_request_id: Option<String>,
    pub execution_kind: RequestExecutionKind,
    pub status: JobStatus,
    pub cancelled: bool,
    pub start_time: DateTime<Utc>,
    pub end_time: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub queue_wait_ms: u64,
    pub interrupted_at: Option<DateTime<Utc>>,
    pub interruption_reason: Option<String>,
    pub recovered_at: Option<DateTime<Utc>>,
    pub recovery_reason: Option<String>,
    pub failed_at: Option<DateTime<Utc>>,
    pub failure_reason: Option<String>,
    pub usage: RequestUsage,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequestHistory {
    pub requests: Vec<RequestSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JobDetail {
    pub job_id: String,
    pub kind: JobKind,
    pub status: JobStatus,
    pub message: Option<String>,
    pub conversation_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub input_text: Option<String>,
    pub output_text: Option<String>,
    pub current_url: Option<String>,
    pub output_path: Option<String>,
    pub artifacts: Vec<ArtifactSummary>,
    pub cancel_requested: bool,
    pub interrupted_at: Option<DateTime<Utc>>,
    pub interruption_reason: Option<String>,
    pub recovered_at: Option<DateTime<Utc>>,
    pub recovery_reason: Option<String>,
    pub failed_at: Option<DateTime<Utc>>,
    pub failure_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AskResult {
    pub job_id: String,
    pub conversation_id: String,
    pub message_id: String,
    pub output_text: String,
    pub artifacts: Vec<ArtifactMetadata>,
    pub completed_at: DateTime<Utc>,
    pub current_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResearchResult {
    pub job_id: String,
    pub conversation_id: String,
    pub message_id: String,
    pub report_text: String,
    pub artifacts: Vec<ArtifactMetadata>,
    pub completed_at: DateTime<Utc>,
    pub current_url: String,
    pub output_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactSummary {
    pub artifact_id: String,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub file_name: String,
    pub content_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub storage_path: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactList {
    pub artifacts: Vec<ArtifactSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPullResult {
    pub artifact: ArtifactSummary,
    pub destination_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DaemonErrorResponse {
    pub error_code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BrowserOpenRequest {
    pub target_url: Option<String>,
    pub browser_path: Option<String>,
    pub headless: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BrowserStatusSnapshot {
    pub running: bool,
    pub browser_kind: String,
    pub profile_dir: String,
    #[serde(default)]
    pub attach_profile_dir: Option<String>,
    #[serde(default)]
    pub attach_profile_note: Option<String>,
    pub downloads_dir: String,
    pub target_url: String,
    pub current_url: Option<String>,
    pub page_title: Option<String>,
    pub page_ready: bool,
    pub authenticated: bool,
    pub reload_count: u64,
    pub screenshot_path: Option<String>,
    pub browser_path: Option<String>,
    pub extension_connected: bool,
    pub capability_report: Option<CapabilityReport>,
    #[serde(default)]
    pub attach_mode: BrowserConnectionMode,
    #[serde(default)]
    pub browser_owner: BrowserOwner,
    #[serde(default)]
    pub cdp_attached: bool,
    #[serde(default)]
    pub target_tab_found: bool,
    #[serde(default)]
    pub target_tab_id: Option<String>,
    #[serde(default)]
    pub target_workspace_key: Option<String>,
    #[serde(default)]
    pub debug_endpoint: Option<String>,
    #[serde(default)]
    pub auth_source: AuthSource,
    #[serde(default)]
    pub extension_required: bool,
    #[serde(default)]
    pub last_capability_report_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatePaths {
    pub state_root: PathBuf,
    pub config_dir: PathBuf,
    pub config_path: PathBuf,
    pub logs_dir: PathBuf,
    pub log_path: PathBuf,
    pub db_dir: PathBuf,
    pub db_path: PathBuf,
    pub browser_profile_dir: PathBuf,
    pub chrome_attach_profile_dir: PathBuf,
    pub downloads_dir: PathBuf,
    pub crash_bundles_dir: PathBuf,
    pub lock_path: PathBuf,
    pub ipc_endpoint: String,
}

pub fn resolve_state_root(explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }

    if let Ok(value) = env::var("TABLINQ_STATE_ROOT") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    default_state_root()
}

pub fn default_state_root() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(value) = env::var("LOCALAPPDATA") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed).join(APP_NAME);
            }
        }

        if let Ok(value) = env::var("USERPROFILE") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed)
                    .join("AppData")
                    .join("Local")
                    .join(APP_NAME);
            }
        }
    }

    #[cfg(not(windows))]
    {
        if let Ok(value) = env::var("XDG_STATE_HOME") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed).join(APP_NAME.to_lowercase());
            }
        }

        if let Ok(value) = env::var("HOME") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed)
                    .join(".local")
                    .join("state")
                    .join(APP_NAME.to_lowercase());
            }
        }
    }

    PathBuf::from(".tablinq")
}

pub fn build_state_paths(state_root: impl AsRef<Path>) -> StatePaths {
    let state_root = state_root.as_ref().to_path_buf();
    let config_dir = state_root.join("config");
    let logs_dir = state_root.join("logs");
    let db_dir = state_root.join("db");
    let browser_profile_dir = state_root.join("browser-profile");
    let chrome_attach_profile_dir = state_root.join(CHROME_ATTACH_PROFILE_DIR_NAME);
    let downloads_dir = state_root.join("downloads");
    let crash_bundles_dir = state_root.join("crash-bundles");

    StatePaths {
        config_path: config_dir.join("config.toml"),
        log_path: logs_dir.join("gptd.log"),
        db_path: db_dir.join("tablinq.sqlite3"),
        lock_path: state_root.join("daemon.lock"),
        ipc_endpoint: ipc_endpoint_for_root(&state_root),
        state_root,
        config_dir,
        logs_dir,
        db_dir,
        browser_profile_dir,
        chrome_attach_profile_dir,
        downloads_dir,
        crash_bundles_dir,
    }
}

pub fn ipc_endpoint_for_root(state_root: impl AsRef<Path>) -> String {
    let input = state_root.as_ref().to_string_lossy();
    format!("tablinq-{:016x}", stable_hash(input.as_bytes()))
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("{0} must not be empty")]
    EmptyField(&'static str),
    #[error("{0} must contain at least one item")]
    EmptyCollection(&'static str),
    #[error("{field} is invalid: {message}")]
    InvalidFieldCombination { field: String, message: String },
    #[error("selector pack format version {found} is unsupported; expected {expected}")]
    UnsupportedSelectorPackFormat { found: u32, expected: u32 },
    #[error("{field} contains duplicate strategy id {strategy_id}")]
    DuplicateStrategy { field: String, strategy_id: String },
    #[error("protocol version {found} is unsupported; expected {expected}")]
    UnsupportedVersion {
        found: String,
        expected: &'static str,
    },
}

fn validate_exact_version(version: &str) -> Result<(), ValidationError> {
    if version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ValidationError::UnsupportedVersion {
            found: version.to_owned(),
            expected: PROTOCOL_VERSION,
        })
    }
}

fn validate_required(field: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() {
        Err(ValidationError::EmptyField(field))
    } else {
        Ok(())
    }
}

fn validate_optional(field: &'static str, value: Option<&str>) -> Result<(), ValidationError> {
    match value {
        Some(value) => validate_required(field, value),
        None => Ok(()),
    }
}

fn validate_paths(field: &'static str, values: &[String]) -> Result<(), ValidationError> {
    for value in values {
        validate_required(field, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_time() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 12, 3, 4, 5).unwrap()
    }

    #[test]
    fn command_envelope_round_trips() {
        let envelope = CommandEnvelope {
            version: PROTOCOL_VERSION.to_owned(),
            request_id: "req-123".to_owned(),
            session_id: Some("session-abc".to_owned()),
            sent_at: fixed_time(),
            command: CommandPayload::Ask(AskCommand {
                prompt: "hello".to_owned(),
                conversation_id: Some("conv-1".to_owned()),
                new_conversation: false,
                mode: Mode::Search,
                model_class: Some(ModelClass::Thinking),
                effort: Some(Effort::Standard),
                files: vec!["fixtures/a.pdf".to_owned()],
            }),
        };

        let json = serde_json::to_string_pretty(&envelope).unwrap();
        let restored: CommandEnvelope = serde_json::from_str(&json).unwrap();

        assert_eq!(restored, envelope);
        assert!(json.contains("\"type\": \"ask\""));
        envelope.validate().unwrap();
    }

    #[test]
    fn event_envelope_round_trips() {
        let envelope = EventEnvelope {
            version: PROTOCOL_VERSION.to_owned(),
            request_id: Some("req-123".to_owned()),
            emitted_at: fixed_time(),
            event: EventPayload::CapabilityReport(CapabilityReport {
                reported_at: fixed_time(),
                selector_pack_id: "chatgpt-default".to_owned(),
                selector_pack_version: "v1".to_owned(),
                matrix: CapabilityMatrix {
                    composer_ready: true,
                    submit_available: true,
                    model_picker_visible: true,
                    thinking_effort_visible: true,
                    search_mode_visible: true,
                    research_mode_visible: false,
                    upload_control_visible: true,
                    downloadable_artifacts_observable: false,
                },
                aliases: RuntimeAliases {
                    modes: BTreeMap::from([
                        ("normal".to_owned(), vec!["Chat".to_owned()]),
                        ("search".to_owned(), vec!["Search".to_owned()]),
                    ]),
                    model_classes: BTreeMap::from([(
                        "thinking".to_owned(),
                        vec!["GPT-5.4 Thinking".to_owned()],
                    )]),
                    efforts: BTreeMap::from([("standard".to_owned(), vec!["Standard".to_owned()])]),
                },
            }),
        };

        let json = serde_json::to_string_pretty(&envelope).unwrap();
        let restored: EventEnvelope = serde_json::from_str(&json).unwrap();

        assert_eq!(restored, envelope);
        assert!(json.contains("\"type\": \"capability_report\""));
        envelope.validate().unwrap();
    }

    #[test]
    fn rejects_blank_request_id() {
        let envelope = CommandEnvelope {
            version: PROTOCOL_VERSION.to_owned(),
            request_id: "   ".to_owned(),
            session_id: None,
            sent_at: fixed_time(),
            command: CommandPayload::HealthPing(HealthPingCommand::default()),
        };

        let err = envelope.validate().unwrap_err();
        assert_eq!(err, ValidationError::EmptyField("request_id"));
    }

    #[test]
    fn selector_pack_validation_rejects_duplicate_strategy_ids() {
        let pack = SelectorPack {
            format_version: SELECTOR_PACK_FORMAT_VERSION,
            pack_id: "chatgpt-default".to_owned(),
            description: "baseline".to_owned(),
            updated_at: fixed_time(),
            capabilities: BTreeMap::from([(
                "composer_ready".to_owned(),
                vec![
                    SelectorStrategy {
                        strategy_id: "composer-role".to_owned(),
                        kind: StrategyKind::AriaRole,
                        selector: "[role='textbox']".to_owned(),
                        score: 0.98,
                    },
                    SelectorStrategy {
                        strategy_id: "composer-role".to_owned(),
                        kind: StrategyKind::Css,
                        selector: "textarea".to_owned(),
                        score: 0.7,
                    },
                ],
            )]),
        };

        let err = pack.validate().unwrap_err();
        assert_eq!(
            err,
            ValidationError::DuplicateStrategy {
                field: "composer_ready".to_owned(),
                strategy_id: "composer-role".to_owned(),
            }
        );
    }

    #[test]
    fn browser_status_snapshot_round_trips_with_attach_fields() {
        let snapshot = BrowserStatusSnapshot {
            running: true,
            browser_kind: "msedge".to_owned(),
            profile_dir: "C:/tablinq/browser-profile".to_owned(),
            attach_profile_dir: Some("C:/tablinq/chrome-attach-profile".to_owned()),
            attach_profile_note: Some(CHROME_ATTACH_PROFILE_NOTE.to_owned()),
            downloads_dir: "C:/tablinq/downloads".to_owned(),
            target_url: "https://chatgpt.com/".to_owned(),
            current_url: Some("https://chatgpt.com/c/conv-1".to_owned()),
            page_title: Some("ChatGPT".to_owned()),
            page_ready: true,
            authenticated: true,
            reload_count: 2,
            screenshot_path: Some("C:/tablinq/crash-bundles/last.png".to_owned()),
            browser_path: Some("C:/Program Files/Microsoft/Edge/Application/msedge.exe".to_owned()),
            extension_connected: true,
            capability_report: None,
            attach_mode: BrowserConnectionMode::AttachFirst,
            browser_owner: BrowserOwner::Managed,
            cdp_attached: false,
            target_tab_found: true,
            target_tab_id: Some("tab-1".to_owned()),
            target_workspace_key: Some("https://chatgpt.com/".to_owned()),
            debug_endpoint: Some("http://127.0.0.1:9222".to_owned()),
            auth_source: AuthSource::Page,
            extension_required: true,
            last_capability_report_at: Some("2026-04-12T10:00:00Z".to_owned()),
        };

        let json = serde_json::to_string_pretty(&snapshot).unwrap();
        let restored: BrowserStatusSnapshot = serde_json::from_str(&json).unwrap();

        assert_eq!(restored, snapshot);
        assert!(json.contains("\"attach_mode\": \"attach_first\""));
        assert!(json.contains("\"attach_profile_dir\": \"C:/tablinq/chrome-attach-profile\""));
        assert!(json.contains("\"browser_owner\": \"managed\""));
        assert!(json.contains("\"auth_source\": \"page\""));
        assert!(json.contains("\"target_workspace_key\": \"https://chatgpt.com/\""));
    }

    #[test]
    fn build_state_paths_includes_dedicated_chrome_attach_profile_dir() {
        let root = PathBuf::from("state-root");
        let paths = build_state_paths(&root);

        assert_eq!(
            paths.chrome_attach_profile_dir,
            root.join("chrome-attach-profile")
        );
        assert!(paths.chrome_attach_profile_dir.starts_with(&root));
        assert_ne!(paths.chrome_attach_profile_dir, paths.browser_profile_dir);
    }
}
