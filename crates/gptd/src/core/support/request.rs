#![allow(unused_imports)]
use super::super::{
    CoreService, ExecutionRecoveryState, recovery::is_provider_generation_failure_message,
};
use crate::browser::{
    BrowserDiagnosticsSnapshot, BrowserManager, RequestLifecycleUpdate, ResearchUpdate,
};
use crate::config::AppConfig;
use crate::db::Database;
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use shared_protocol::{
    BrowserConnectionMode, BrowserStatusSnapshot, ErrorCode, EventPayload, HealthCheck,
    HealthStatus, JobState, JobStatus, ResearchCommand, SelectorPackSummary, StreamChunk,
    StreamChunkKind,
};
use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use url::Url;

impl CoreService {
    pub(in crate::core) fn open_db(&self) -> Result<Database> {
        Database::open(&self.config.paths.db_path)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::core) fn apply_request_lifecycle_update<F>(
        &self,
        db: &Database,
        job_id: &str,
        created_at: DateTime<Utc>,
        conversation_id: Option<&str>,
        update: RequestLifecycleUpdate,
        recovery_state: &RefCell<ExecutionRecoveryState>,
        on_event: &RefCell<&mut F>,
    ) -> Result<()>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let updated_at = Utc::now();
        match update.status {
            JobStatus::Interrupted => {
                recovery_state.borrow_mut().interrupted = true;
                db.insert_job(
                    job_id,
                    JobStatus::Interrupted,
                    Some(&update.message),
                    update.conversation_id.as_deref().or(conversation_id),
                    created_at,
                    updated_at,
                )?;
                db.record_job_interruption(
                    job_id,
                    updated_at,
                    &update.message,
                    update.current_url.as_deref(),
                )?;
            }
            JobStatus::Recovered => {
                recovery_state.borrow_mut().recovered = true;
                db.insert_job(
                    job_id,
                    JobStatus::Recovered,
                    Some(&update.message),
                    update.conversation_id.as_deref().or(conversation_id),
                    created_at,
                    updated_at,
                )?;
                db.record_job_recovery(
                    job_id,
                    updated_at,
                    &update.message,
                    update.current_url.as_deref(),
                )?;
            }
            _ => return Ok(()),
        }

        let mut sink = on_event.borrow_mut();
        (*sink)(EventPayload::JobState(JobState {
            job_id: job_id.to_owned(),
            status: update.status,
            message: Some(update.message),
            conversation_id: update.conversation_id,
            updated_at,
            artifacts: vec![],
        }))
    }
}
