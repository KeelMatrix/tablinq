use crate::{Effort, ErrorCode, Mode, ModelClass};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryStrategy {
    ReloadNewConversationReplayWithContext,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryManifestFileKind {
    InputAttachment,
    GeneratedArtifact,
    RecoveryManifestAttachment,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryManifestFile {
    pub kind: RecoveryManifestFileKind,
    pub file_name: String,
    pub local_path: String,
    pub content_type: Option<String>,
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryManifestTurn {
    pub completed_at: DateTime<Utc>,
    pub request_text: String,
    pub response_excerpt: String,
    pub input_files: Vec<String>,
    pub generated_artifacts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationRecoveryManifest {
    pub conversation_id: String,
    pub archived_context: Vec<String>,
    pub recent_turns: Vec<RecoveryManifestTurn>,
    pub tracked_input_files: Vec<RecoveryManifestFile>,
    pub tracked_generated_artifacts: Vec<RecoveryManifestFile>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryTrace {
    pub recovery_available: bool,
    pub recovery_attempted: bool,
    pub recovery_strategy: Option<RecoveryStrategy>,
    pub failure_code: Option<ErrorCode>,
    pub failure_bundle_id: Option<String>,
    pub prior_request_id: Option<String>,
    pub replacement_request_id: Option<String>,
    pub prior_conversation_id: Option<String>,
    pub replacement_conversation_id: Option<String>,
    pub recovery_attempt_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RecoveryPacket {
    pub original_input: String,
    pub mode: Mode,
    pub model_class: Option<ModelClass>,
    pub effort: Option<Effort>,
    pub files: Vec<String>,
    pub context_summary: Option<String>,
    #[serde(default)]
    pub context_manifest: Option<ConversationRecoveryManifest>,
    #[serde(default)]
    pub recovery_manifest_path: Option<String>,
    #[serde(default)]
    pub replay_files: Vec<String>,
    pub prior_request_id: Option<String>,
    pub prior_conversation_id: Option<String>,
    pub failure_code: ErrorCode,
    pub recovery_attempt_count: u32,
}
