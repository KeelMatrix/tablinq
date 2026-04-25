mod artifacts;
mod conversations;
mod failure_bundles;
mod jobs;
mod migrations;
mod requests;
mod row_mapping;
mod sessions;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use shared_protocol::{
    ArtifactList, ArtifactMetadata, ArtifactSummary, ConversationList,
    ConversationRecoveryManifest, ConversationSessionInfo, ConversationSummary, FailureBundle,
    JobDetail, JobKind, JobList, JobStatus, RecoveryPacket, RecoveryTrace, RequestHistory,
    RequestListPage, RequestSummary, SessionList,
};
use std::path::Path;

const REQUEST_HISTORY_LIMIT: usize = 200;
#[cfg(test)]
const ARTIFACT_HISTORY_LIMIT: usize = 100;

pub struct Database {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct ConversationTurnRecord {
    pub job: JobDetail,
    pub request: Option<RequestSummary>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        migrations::apply(&conn)?;
        Ok(Self { conn })
    }

    pub fn migration_count(&self) -> Result<u32> {
        migrations::count(&self.conn)
    }

    pub fn list_sessions(&self) -> Result<SessionList> {
        sessions::list(&self.conn)
    }

    pub fn find_conversation(&self, conversation_id: &str) -> Result<Option<ConversationSummary>> {
        conversations::find(&self.conn, conversation_id)
    }

    pub fn find_session_info(&self, session_id: &str) -> Result<Option<ConversationSessionInfo>> {
        sessions::find_info(&self.conn, session_id)
    }

    pub fn conversation_recovery_manifest(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationRecoveryManifest>> {
        conversations::recovery_manifest(&self.conn, conversation_id)
    }

    pub fn list_conversations(&self) -> Result<ConversationList> {
        conversations::list(&self.conn)
    }

    pub fn list_session_conversations(&self, session_id: &str) -> Result<Vec<ConversationSummary>> {
        conversations::list_for_session(&self.conn, session_id)
    }

    pub fn list_conversation_turn_records(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ConversationTurnRecord>> {
        conversations::list_turn_records(&self.conn, conversation_id)
    }

    pub fn list_conversation_artifacts(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<ArtifactSummary>> {
        conversations::list_artifacts(&self.conn, conversation_id)
    }

    pub fn list_jobs(&self) -> Result<JobList> {
        jobs::list(&self.conn)
    }

    pub fn upsert_request_summary(&self, summary: &RequestSummary) -> Result<()> {
        requests::upsert_summary(&self.conn, summary)
    }

    pub fn request_summary(&self, request_id: &str) -> Result<Option<RequestSummary>> {
        requests::find_summary(&self.conn, request_id)
    }

    pub fn list_request_summaries(&self, limit: usize) -> Result<RequestHistory> {
        requests::list_summaries(&self.conn, limit)
    }

    pub fn list_request_page(
        &self,
        status: Option<JobStatus>,
        limit: usize,
        offset: usize,
    ) -> Result<RequestListPage> {
        requests::list_page(&self.conn, status, limit, offset)
    }

    pub fn upsert_session(
        &self,
        session_id: &str,
        label: &str,
        tab_url: Option<&str>,
        last_seen_at: DateTime<Utc>,
    ) -> Result<()> {
        sessions::upsert(&self.conn, session_id, label, tab_url, last_seen_at)
    }

    pub fn insert_job(
        &self,
        job_id: &str,
        status: JobStatus,
        message: Option<&str>,
        conversation_id: Option<&str>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Result<()> {
        jobs::insert(
            &self.conn,
            job_id,
            status,
            message,
            conversation_id,
            created_at,
            updated_at,
        )
    }

    pub fn upsert_job_detail(
        &self,
        job_id: &str,
        kind: JobKind,
        input_text: Option<&str>,
        output_path: Option<&str>,
    ) -> Result<()> {
        jobs::upsert_detail(&self.conn, job_id, kind, input_text, output_path)
    }

    pub fn update_job_output(
        &self,
        job_id: &str,
        output_text: Option<&str>,
        current_url: Option<&str>,
        output_path: Option<&str>,
    ) -> Result<()> {
        jobs::update_output(&self.conn, job_id, output_text, current_url, output_path)
    }

    pub fn set_job_cancel_requested(&self, job_id: &str, cancel_requested: bool) -> Result<()> {
        jobs::set_cancel_requested(&self.conn, job_id, cancel_requested)
    }

    pub fn job_cancel_requested(&self, job_id: &str) -> Result<bool> {
        jobs::cancel_requested(&self.conn, job_id)
    }

    pub fn record_job_interruption(
        &self,
        job_id: &str,
        interrupted_at: DateTime<Utc>,
        interruption_reason: &str,
        current_url: Option<&str>,
    ) -> Result<()> {
        jobs::record_interruption(
            &self.conn,
            job_id,
            interrupted_at,
            interruption_reason,
            current_url,
        )
    }

    pub fn record_job_recovery(
        &self,
        job_id: &str,
        recovered_at: DateTime<Utc>,
        recovery_reason: &str,
        current_url: Option<&str>,
    ) -> Result<()> {
        jobs::record_recovery(
            &self.conn,
            job_id,
            recovered_at,
            recovery_reason,
            current_url,
        )
    }

    pub fn record_job_failure(
        &self,
        job_id: &str,
        failed_at: DateTime<Utc>,
        failure_reason: &str,
        current_url: Option<&str>,
    ) -> Result<()> {
        jobs::record_failure(&self.conn, job_id, failed_at, failure_reason, current_url)
    }

    pub fn update_job_recovery_metadata(
        &self,
        job_id: &str,
        recovery_trace: Option<&RecoveryTrace>,
        recovery_packet: Option<&RecoveryPacket>,
    ) -> Result<()> {
        jobs::update_recovery_metadata(&self.conn, job_id, recovery_trace, recovery_packet)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn upsert_conversation(
        &self,
        conversation_id: &str,
        session_id: &str,
        label: &str,
        tab_url: Option<&str>,
        last_prompt: Option<&str>,
        last_response: Option<&str>,
        recovery_manifest: Option<&ConversationRecoveryManifest>,
        last_seen_at: DateTime<Utc>,
    ) -> Result<()> {
        conversations::upsert(
            &self.conn,
            conversation_id,
            session_id,
            label,
            tab_url,
            last_prompt,
            last_response,
            recovery_manifest,
            last_seen_at,
        )
    }

    pub fn latest_conversation_id(&self) -> Result<Option<String>> {
        conversations::latest_id(&self.conn)
    }

    pub fn insert_artifact(
        &self,
        artifact: &ArtifactMetadata,
        created_at: DateTime<Utc>,
    ) -> Result<()> {
        artifacts::insert(&self.conn, artifact, created_at)
    }

    pub fn prune_artifacts(&self, limit: usize) -> Result<usize> {
        artifacts::prune(&self.conn, limit)
    }

    pub fn link_job_artifact(&self, job_id: &str, artifact_id: &str) -> Result<()> {
        artifacts::link_job(&self.conn, job_id, artifact_id)
    }

    pub fn list_artifacts(&self) -> Result<ArtifactList> {
        artifacts::list(&self.conn)
    }

    pub fn latest_artifact(&self) -> Result<Option<ArtifactSummary>> {
        artifacts::latest(&self.conn)
    }

    pub fn latest_request_artifact_by_content_type(
        &self,
        request_id: &str,
        content_type: &str,
    ) -> Result<Option<ArtifactSummary>> {
        artifacts::latest_request_by_content_type(&self.conn, request_id, content_type)
    }

    pub fn find_artifact(&self, artifact_id: &str) -> Result<Option<ArtifactSummary>> {
        artifacts::find(&self.conn, artifact_id)
    }

    pub fn artifact_job_ids(&self, artifact_id: &str) -> Result<Vec<String>> {
        artifacts::job_ids(&self.conn, artifact_id)
    }

    pub fn artifact_request_ids(&self, artifact_id: &str) -> Result<Vec<String>> {
        artifacts::request_ids(&self.conn, artifact_id)
    }

    pub fn job_detail(&self, job_id: &str) -> Result<Option<JobDetail>> {
        jobs::detail(&self.conn, job_id)
    }

    pub fn fail_incomplete_jobs(&self, message: &str, updated_at: DateTime<Utc>) -> Result<()> {
        jobs::fail_incomplete(&self.conn, message, updated_at)
    }

    pub fn insert_failure_bundle(&self, bundle: &FailureBundle) -> Result<()> {
        failure_bundles::insert(&self.conn, bundle)
    }

    pub fn count_failure_bundles(&self) -> Result<u64> {
        failure_bundles::count(&self.conn)
    }

    pub fn latest_failure_bundle(&self) -> Result<Option<FailureBundle>> {
        failure_bundles::latest(&self.conn)
    }
}

#[cfg(test)]
mod tests;
