use crate::{JobStatus, RecoveryPacket, RecoveryTrace};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestExecutionKind {
    Ask,
    Research,
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
    #[serde(default)]
    pub recovery_trace: Option<RecoveryTrace>,
    #[serde(default)]
    pub recovery_packet: Option<RecoveryPacket>,
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
pub struct RequestListMetadata {
    pub job_id: String,
    pub request_role: Option<String>,
    pub parent_request_id: Option<String>,
    pub execution_kind: RequestExecutionKind,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequestListEntry {
    pub id: String,
    pub status: JobStatus,
    pub created_at: DateTime<Utc>,
    pub model: Option<String>,
    pub metadata: RequestListMetadata,
}

impl From<RequestSummary> for RequestListEntry {
    fn from(summary: RequestSummary) -> Self {
        Self {
            id: summary.request_id,
            status: summary.status,
            created_at: summary.start_time,
            model: summary.model_id,
            metadata: RequestListMetadata {
                job_id: summary.job_id,
                request_role: summary.request_role,
                parent_request_id: summary.parent_request_id,
                execution_kind: summary.execution_kind,
                cancelled: summary.cancelled,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RequestListPage {
    pub requests: Vec<RequestListEntry>,
    pub limit: usize,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub status: Option<JobStatus>,
}
