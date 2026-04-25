use crate::validation::{ValidationError, validate_optional, validate_required};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
