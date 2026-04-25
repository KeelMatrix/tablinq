use crate::{ArtifactMetadata, ArtifactSummary, RecoveryPacket, RecoveryTrace};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

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

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Queued => "queued",
            JobStatus::Starting => "starting",
            JobStatus::Running => "running",
            JobStatus::Interrupted => "interrupted",
            JobStatus::Recovered => "recovered",
            JobStatus::WaitingForSources => "waiting_for_sources",
            JobStatus::Completed => "completed",
            JobStatus::Cancelled => "cancelled",
            JobStatus::Failed => "failed",
            JobStatus::ArtifactReady => "artifact_ready",
        }
    }
}

impl fmt::Display for JobStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for JobStatus {
    type Err = ();

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "queued" => Ok(JobStatus::Queued),
            "starting" => Ok(JobStatus::Starting),
            "running" => Ok(JobStatus::Running),
            "interrupted" => Ok(JobStatus::Interrupted),
            "recovered" => Ok(JobStatus::Recovered),
            "waiting_for_sources" => Ok(JobStatus::WaitingForSources),
            "completed" => Ok(JobStatus::Completed),
            "cancelled" => Ok(JobStatus::Cancelled),
            "failed" => Ok(JobStatus::Failed),
            "artifact_ready" => Ok(JobStatus::ArtifactReady),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Ask,
    Research,
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
    #[serde(default)]
    pub recovery_trace: Option<RecoveryTrace>,
    #[serde(default)]
    pub recovery_packet: Option<RecoveryPacket>,
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
    #[serde(default)]
    pub low_confidence: bool,
    #[serde(default)]
    pub recovery_trace: Option<RecoveryTrace>,
    #[serde(default)]
    pub recovery_packet: Option<RecoveryPacket>,
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
    #[serde(default)]
    pub low_confidence: bool,
}
