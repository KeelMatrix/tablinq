use crate::{
    BrowserStatusSnapshot, ConversationRecoveryManifest, RecoveryManifestFile, RecoveryPacket,
    RecoveryTrace,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
pub struct ConversationList {
    pub conversations: Vec<ConversationSummary>,
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
pub struct ConversationSelection {
    pub conversation_id: Option<String>,
    pub session_id: Option<String>,
    pub current_url: Option<String>,
    pub browser: BrowserStatusSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSessionInfo {
    pub session_id: String,
    pub label: String,
    pub tab_url: Option<String>,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationRecoveryManifestSummary {
    pub updated_at: DateTime<Utc>,
    pub archived_context_count: usize,
    pub recent_turn_count: usize,
    pub tracked_input_file_count: usize,
    pub tracked_generated_artifact_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationCounts {
    pub request_count: usize,
    pub artifact_count: usize,
    pub history_item_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationRouteHints {
    pub detail: String,
    pub messages: String,
    pub snapshot: String,
    pub select: String,
    #[serde(default)]
    pub latest_request: Option<String>,
    #[serde(default)]
    pub latest_request_evidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationDetail {
    #[serde(flatten)]
    pub summary: ConversationSummary,
    #[serde(default)]
    pub session: Option<ConversationSessionInfo>,
    #[serde(default)]
    pub latest_request_id: Option<String>,
    #[serde(default)]
    pub latest_job_id: Option<String>,
    #[serde(default)]
    pub recovery_manifest: Option<ConversationRecoveryManifestSummary>,
    pub counts: ConversationCounts,
    pub routes: ConversationRouteHints,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationMessageSourceKind {
    RequestDetail,
    RecoveryManifest,
    SummaryFallback,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationMessageRole {
    User,
    Assistant,
    System,
    Tool,
    Summary,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationMessageExactness {
    Exact,
    Excerpt,
    Summary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationFileReference {
    pub file_name: String,
    #[serde(default)]
    pub artifact_id: Option<String>,
    #[serde(default)]
    pub local_path: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    pub storage_path: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationMessage {
    pub source_kind: ConversationMessageSourceKind,
    pub role: ConversationMessageRole,
    pub exactness: ConversationMessageExactness,
    pub content: String,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub request_id: Option<String>,
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<ConversationFileReference>,
    #[serde(default)]
    pub input_files: Vec<ConversationFileReference>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationMessageList {
    pub conversation_id: String,
    pub messages: Vec<ConversationMessage>,
    pub limit: usize,
    pub offset: usize,
    #[serde(default)]
    pub next_offset: Option<usize>,
    pub total_items: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSnapshot {
    pub conversation: ConversationDetail,
    #[serde(default)]
    pub context_summary: Option<String>,
    #[serde(default)]
    pub recovery_manifest: Option<ConversationRecoveryManifest>,
    #[serde(default)]
    pub recent_messages: Vec<ConversationMessage>,
    #[serde(default)]
    pub tracked_input_files: Vec<RecoveryManifestFile>,
    #[serde(default)]
    pub tracked_generated_artifacts: Vec<RecoveryManifestFile>,
    #[serde(default)]
    pub latest_recovery_packet: Option<RecoveryPacket>,
    #[serde(default)]
    pub latest_recovery_trace: Option<RecoveryTrace>,
    #[serde(default)]
    pub latest_request_id: Option<String>,
    #[serde(default)]
    pub latest_job_id: Option<String>,
    #[serde(default)]
    pub replacement_request_id: Option<String>,
    #[serde(default)]
    pub replacement_conversation_id: Option<String>,
    pub suitable_for_fresh_conversation_replay: bool,
    pub replay_readiness_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionConversationList {
    pub session: ConversationSessionInfo,
    pub conversations: Vec<ConversationSummary>,
}
