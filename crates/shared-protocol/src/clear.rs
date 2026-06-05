use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ClearStateResult {
    pub sessions_deleted: u64,
    pub conversations_deleted: u64,
    pub jobs_deleted: u64,
    pub job_details_deleted: u64,
    pub request_history_deleted: u64,
    pub artifacts_deleted: u64,
    pub artifact_links_deleted: u64,
    pub failure_bundles_deleted: u64,
    pub native_upload_attempts_deleted: u64,
    pub artifact_files_deleted: u64,
    pub artifact_files_missing: u64,
    pub artifact_files_failed: u64,
}

impl ClearStateResult {
    pub fn database_rows_deleted(&self) -> u64 {
        self.sessions_deleted
            + self.conversations_deleted
            + self.jobs_deleted
            + self.job_details_deleted
            + self.request_history_deleted
            + self.artifacts_deleted
            + self.artifact_links_deleted
            + self.failure_bundles_deleted
            + self.native_upload_attempts_deleted
    }
}
