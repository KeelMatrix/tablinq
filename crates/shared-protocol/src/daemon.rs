use crate::{
    AckEvent, ArtifactList, ArtifactPullResult, AskCommand, AskResult, BrowserOpenRequest,
    BrowserStatusSnapshot, CancelJobCommand, CapabilitySnapshot, ClearStateResult, ConfigSnapshot,
    ConversationList, ConversationSelectCommand, ConversationSelection, DoctorReport, ErrorCode,
    EventPayload, HealthCheck, JobDetail, JobList, PullArtifactCommand, RecoveryPacket,
    RecoveryTrace, ResearchCommand, ResearchResult, SessionList,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    ConversationsList,
    ConversationSelect(ConversationSelectCommand),
    JobsList,
    JobsShow(String),
    JobsCancel(CancelJobCommand),
    ArtifactsList,
    ArtifactPull(PullArtifactCommand),
    ClearAllState,
    BrowserOpen(BrowserOpenRequest),
    BrowserClose,
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
    Conversations(ConversationList),
    ConversationSelection(ConversationSelection),
    Jobs(JobList),
    Job(JobDetail),
    Artifacts(ArtifactList),
    ArtifactPull(ArtifactPullResult),
    ClearState(ClearStateResult),
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
pub struct DaemonErrorResponse {
    pub error_code: ErrorCode,
    pub message: String,
    #[serde(default)]
    pub recovery_trace: Option<RecoveryTrace>,
    #[serde(default)]
    pub recovery_packet: Option<RecoveryPacket>,
}
