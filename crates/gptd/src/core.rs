use crate::browser::{
    BrowserDiagnosticsSnapshot, BrowserManager, RequestLifecycleUpdate, ResearchUpdate,
};
use crate::config::AppConfig;
use crate::db::Database;
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use shared_protocol::{
    AckEvent, ArtifactList, ArtifactPullResult, AskCommand, AskResult, BrowserConnectionMode,
    BrowserDoctor, BrowserOpenRequest, BrowserStatusSnapshot, CancelJobCommand, CapabilitySnapshot,
    ConfigSnapshot, ConversationSummary, DoctorReport, ErrorCode, EventPayload, FailureBundle,
    GatewayRequestContext, HealthCheck, HealthStatus, JobDetail, JobKind, JobList, JobState,
    JobStatus, PullArtifactCommand, RequestHistory, RequestSummary, ResearchCommand,
    ResearchResult, SelectorPackSummary, SessionList, StreamChunk, StreamChunkKind,
};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

const ARTIFACT_RETENTION_LIMIT: usize = 100;

#[derive(Debug, Default)]
struct ExecutionRecoveryState {
    interrupted: bool,
    recovered: bool,
}

#[derive(Clone)]
pub struct CoreService {
    config: AppConfig,
    started_at: chrono::DateTime<Utc>,
    browser: Arc<Mutex<BrowserManager>>,
    cancel_registry: Arc<Mutex<HashSet<String>>>,
}

#[derive(Debug, Clone)]
pub struct PreparedAskJob {
    pub job_id: String,
    pub command: AskCommand,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct PreparedResearchJob {
    pub job_id: String,
    pub command: ResearchCommand,
    pub created_at: DateTime<Utc>,
}

impl CoreService {
    pub fn new(config: AppConfig) -> Result<Self> {
        let db = Database::open(&config.paths.db_path)?;
        db.fail_incomplete_jobs("daemon restarted before the job finished", Utc::now())?;

        Ok(Self {
            config: config.clone(),
            started_at: Utc::now(),
            browser: Arc::new(Mutex::new(BrowserManager::new(&config))),
            cancel_registry: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn warm_browser_if_enabled(&self) -> Result<()> {
        if self.config.warm_mode() {
            lock_browser(&self.browser)?.open(BrowserOpenRequest {
                target_url: None,
                browser_path: None,
                headless: None,
            })?;
        }
        Ok(())
    }

    pub fn ask<F>(&self, command: AskCommand, mut on_event: F) -> Result<AskResult>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let prepared = self.prepare_ask(command)?;
        on_event(EventPayload::JobState(JobState {
            job_id: prepared.job_id.clone(),
            status: JobStatus::Queued,
            message: Some("queued ask job".to_owned()),
            conversation_id: prepared.command.conversation_id.clone(),
            updated_at: prepared.created_at,
            artifacts: vec![],
        }))?;
        self.execute_prepared_ask(&prepared, on_event)
    }

    pub fn research<F>(&self, command: ResearchCommand, mut on_event: F) -> Result<ResearchResult>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let prepared = self.prepare_research(command)?;
        on_event(EventPayload::JobState(JobState {
            job_id: prepared.job_id.clone(),
            status: JobStatus::Queued,
            message: Some("queued research job".to_owned()),
            conversation_id: prepared.command.conversation_id.clone(),
            updated_at: prepared.created_at,
            artifacts: vec![],
        }))?;
        self.execute_prepared_research(&prepared, on_event)
    }

    pub fn prepare_ask(&self, command: AskCommand) -> Result<PreparedAskJob> {
        if self.config.safe_mode() {
            anyhow::bail!(
                "safe mode enabled: prompt execution is disabled; run doctor or capabilities only"
            );
        }

        let db = self.open_db()?;
        let created_at = Utc::now();
        let job_id = format!("job-{}", created_at.timestamp_millis());
        let conversation_id = match (command.new_conversation, command.conversation_id.clone()) {
            (true, _) => None,
            (false, Some(value)) => {
                if db.find_conversation(&value)?.is_none() {
                    anyhow::bail!("conversation {value} was not found");
                }
                Some(value)
            }
            (false, None) => db.latest_conversation_id()?,
        };
        let mut effective = command.clone();
        effective.conversation_id = conversation_id.clone();
        effective.new_conversation = false;

        db.insert_job(
            &job_id,
            JobStatus::Queued,
            Some("queued ask job"),
            effective.conversation_id.as_deref(),
            created_at,
            created_at,
        )?;
        db.upsert_job_detail(&job_id, JobKind::Ask, Some(&effective.prompt), None)?;

        Ok(PreparedAskJob {
            job_id,
            command: effective,
            created_at,
        })
    }

    pub fn execute_prepared_ask<F>(
        &self,
        prepared: &PreparedAskJob,
        mut on_event: F,
    ) -> Result<AskResult>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let db = self.open_db()?;
        if db.job_cancel_requested(&prepared.job_id)? {
            let message = "ask cancelled before execution";
            self.cancel_job_with_detail(
                &prepared.job_id,
                prepared.command.conversation_id.as_deref(),
                message,
            )?;
            on_event(EventPayload::JobState(JobState {
                job_id: prepared.job_id.clone(),
                status: JobStatus::Cancelled,
                message: Some(message.to_owned()),
                conversation_id: prepared.command.conversation_id.clone(),
                updated_at: Utc::now(),
                artifacts: vec![],
            }))?;
            anyhow::bail!(message);
        }

        db.insert_job(
            &prepared.job_id,
            JobStatus::Starting,
            Some("starting ask"),
            prepared.command.conversation_id.as_deref(),
            prepared.created_at,
            Utc::now(),
        )?;
        on_event(EventPayload::JobState(JobState {
            job_id: prepared.job_id.clone(),
            status: JobStatus::Starting,
            message: Some("starting ask".to_owned()),
            conversation_id: prepared.command.conversation_id.clone(),
            updated_at: Utc::now(),
            artifacts: vec![],
        }))?;

        db.insert_job(
            &prepared.job_id,
            JobStatus::Running,
            Some("streaming response"),
            prepared.command.conversation_id.as_deref(),
            prepared.created_at,
            Utc::now(),
        )?;
        on_event(EventPayload::JobState(JobState {
            job_id: prepared.job_id.clone(),
            status: JobStatus::Running,
            message: Some("streaming response".to_owned()),
            conversation_id: prepared.command.conversation_id.clone(),
            updated_at: Utc::now(),
            artifacts: vec![],
        }))?;

        let recovery_state = RefCell::new(ExecutionRecoveryState::default());
        let event_sink = RefCell::new(&mut on_event);
        let outcome = lock_browser(&self.browser)?.ask(
            &prepared.job_id,
            &prepared.command,
            |event| {
                let mut sink = event_sink.borrow_mut();
                (*sink)(event)
            },
            |update| {
                self.apply_request_lifecycle_update(
                    &db,
                    &prepared.job_id,
                    prepared.created_at,
                    prepared.command.conversation_id.as_deref(),
                    update,
                    &recovery_state,
                    &event_sink,
                )
            },
            || {
                lock_cancel_registry(&self.cancel_registry)
                    .map(|set| set.contains(&prepared.job_id))
                    .unwrap_or(false)
            },
        );

        let _ = lock_cancel_registry(&self.cancel_registry).map(|mut set| {
            set.remove(&prepared.job_id);
        });
        let _ = db.set_job_cancel_requested(&prepared.job_id, false);

        match outcome {
            Ok(result) => {
                let recovery_state = recovery_state.into_inner();
                let completed_at = result.completed_at;
                for artifact in &result.artifacts {
                    db.insert_artifact(artifact, completed_at)?;
                    db.link_job_artifact(&prepared.job_id, &artifact.artifact_id)?;
                    on_event(EventPayload::ArtifactReady(artifact.clone()))?;
                }
                db.prune_artifacts(ARTIFACT_RETENTION_LIMIT)?;
                let final_status = if recovery_state.recovered {
                    JobStatus::Recovered
                } else {
                    JobStatus::Completed
                };
                let final_message = if recovery_state.recovered {
                    "ask recovered"
                } else {
                    "ask completed"
                };
                db.insert_job(
                    &prepared.job_id,
                    final_status,
                    Some(final_message),
                    Some(&result.conversation_id),
                    prepared.created_at,
                    completed_at,
                )?;
                if recovery_state.recovered {
                    db.record_job_recovery(
                        &prepared.job_id,
                        completed_at,
                        "reconciled the interrupted ask against the live conversation",
                        Some(&result.current_url),
                    )?;
                }
                let browser_snapshot = lock_browser(&self.browser)?.health()?;
                let (session_id, session_label) = browser_session_identity(&browser_snapshot);
                db.upsert_session(
                    &session_id,
                    &session_label,
                    Some(&result.current_url),
                    completed_at,
                )?;
                db.upsert_conversation(
                    &result.conversation_id,
                    &session_id,
                    "Browser conversation",
                    Some(&result.current_url),
                    Some(&prepared.command.prompt),
                    Some(&result.output_text),
                    completed_at,
                )?;
                db.update_job_output(
                    &prepared.job_id,
                    Some(&result.output_text),
                    Some(&result.current_url),
                    None,
                )?;
                on_event(EventPayload::JobState(JobState {
                    job_id: prepared.job_id.clone(),
                    status: final_status,
                    message: Some(final_message.to_owned()),
                    conversation_id: Some(result.conversation_id.clone()),
                    updated_at: completed_at,
                    artifacts: result.artifacts.clone(),
                }))?;
                Ok(result)
            }
            Err(err) => {
                let failed_at = Utc::now();
                let message = format!("{err:#}");
                let status = if is_cancelled_message(&message) {
                    JobStatus::Cancelled
                } else {
                    JobStatus::Failed
                };
                db.insert_job(
                    &prepared.job_id,
                    status,
                    Some(&message),
                    prepared.command.conversation_id.as_deref(),
                    prepared.created_at,
                    failed_at,
                )?;
                db.record_job_failure(&prepared.job_id, failed_at, &message, None)?;
                db.update_job_output(&prepared.job_id, None, None, None)?;
                on_event(EventPayload::JobState(JobState {
                    job_id: prepared.job_id.clone(),
                    status,
                    message: Some(message),
                    conversation_id: prepared.command.conversation_id.clone(),
                    updated_at: failed_at,
                    artifacts: vec![],
                }))?;
                Err(err)
            }
        }
    }

    pub fn prepare_research(&self, command: ResearchCommand) -> Result<PreparedResearchJob> {
        if self.config.safe_mode() {
            anyhow::bail!(
                "safe mode enabled: prompt execution is disabled; run doctor or capabilities only"
            );
        }

        let db = self.open_db()?;
        let created_at = Utc::now();
        let job_id = format!("job-{}", created_at.timestamp_millis());
        let conversation_id = match (command.new_conversation, command.conversation_id.clone()) {
            (true, _) => None,
            (false, Some(value)) => {
                if db.find_conversation(&value)?.is_none() {
                    anyhow::bail!("conversation {value} was not found");
                }
                Some(value)
            }
            (false, None) => db.latest_conversation_id()?,
        };
        let mut effective = command.clone();
        effective.conversation_id = conversation_id.clone();
        effective.new_conversation = false;
        db.insert_job(
            &job_id,
            JobStatus::Queued,
            Some("queued research job"),
            conversation_id.as_deref(),
            created_at,
            created_at,
        )?;
        db.upsert_job_detail(&job_id, JobKind::Research, Some(&effective.query), None)?;

        Ok(PreparedResearchJob {
            job_id,
            command: effective,
            created_at,
        })
    }

    pub fn execute_prepared_research<F>(
        &self,
        prepared: &PreparedResearchJob,
        mut on_event: F,
    ) -> Result<ResearchResult>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let db = self.open_db()?;
        if db.job_cancel_requested(&prepared.job_id)? {
            let message = "research cancelled before execution";
            self.cancel_job_with_detail(
                &prepared.job_id,
                prepared.command.conversation_id.as_deref(),
                message,
            )?;
            on_event(EventPayload::JobState(JobState {
                job_id: prepared.job_id.clone(),
                status: JobStatus::Cancelled,
                message: Some(message.to_owned()),
                conversation_id: prepared.command.conversation_id.clone(),
                updated_at: Utc::now(),
                artifacts: vec![],
            }))?;
            anyhow::bail!(message);
        }

        let conversation_id = RefCell::new(None);
        let current_url = RefCell::new(None);
        let report_text = RefCell::new(String::new());
        let next_seq = Cell::new(0u64);
        let recovery_state = RefCell::new(ExecutionRecoveryState::default());
        let event_sink = RefCell::new(&mut on_event);

        let outcome = lock_browser(&self.browser)?.research(
            &prepared.job_id,
            &prepared.command,
            |update| {
                let mut conversation_id_ref = conversation_id.borrow_mut();
                let mut current_url_ref = current_url.borrow_mut();
                let mut report_text_ref = report_text.borrow_mut();
                let mut next_seq_value = next_seq.get();
                apply_research_update(
                    &db,
                    &prepared.job_id,
                    prepared.created_at,
                    &mut conversation_id_ref,
                    &mut current_url_ref,
                    &mut report_text_ref,
                    &mut next_seq_value,
                    update,
                    &mut |event| {
                        let mut sink = event_sink.borrow_mut();
                        (*sink)(event)
                    },
                )?;
                next_seq.set(next_seq_value);
                Ok(())
            },
            |update| {
                self.apply_request_lifecycle_update(
                    &db,
                    &prepared.job_id,
                    prepared.created_at,
                    conversation_id
                        .borrow()
                        .as_deref()
                        .or(prepared.command.conversation_id.as_deref()),
                    update,
                    &recovery_state,
                    &event_sink,
                )
            },
            || {
                lock_cancel_registry(&self.cancel_registry)
                    .map(|set| set.contains(&prepared.job_id))
                    .unwrap_or(false)
            },
        );

        let _ = lock_cancel_registry(&self.cancel_registry).map(|mut set| {
            set.remove(&prepared.job_id);
        });
        let _ = db.set_job_cancel_requested(&prepared.job_id, false);

        match outcome {
            Ok(mut result) => {
                let recovery_state = recovery_state.into_inner();
                let output_path = maybe_write_research_output(
                    &prepared.command,
                    &prepared.job_id,
                    &result.report_text,
                )?;
                result.output_path = output_path.clone();
                let completed_at = result.completed_at;
                for artifact in &result.artifacts {
                    db.insert_artifact(artifact, completed_at)?;
                    db.link_job_artifact(&prepared.job_id, &artifact.artifact_id)?;
                    on_event(EventPayload::ArtifactReady(artifact.clone()))?;
                }
                db.prune_artifacts(ARTIFACT_RETENTION_LIMIT)?;
                let final_status = if recovery_state.recovered {
                    JobStatus::Recovered
                } else {
                    JobStatus::Completed
                };
                let final_message = if recovery_state.recovered {
                    "research recovered"
                } else {
                    "research completed"
                };
                db.insert_job(
                    &prepared.job_id,
                    final_status,
                    Some(final_message),
                    Some(&result.conversation_id),
                    prepared.created_at,
                    completed_at,
                )?;
                if recovery_state.recovered {
                    db.record_job_recovery(
                        &prepared.job_id,
                        completed_at,
                        "reconciled the interrupted research job against the live conversation",
                        Some(&result.current_url),
                    )?;
                }
                db.update_job_output(
                    &prepared.job_id,
                    Some(&result.report_text),
                    Some(&result.current_url),
                    output_path.as_deref(),
                )?;
                let browser_snapshot = lock_browser(&self.browser)?.health()?;
                let (session_id, session_label) = browser_session_identity(&browser_snapshot);
                db.upsert_session(
                    &session_id,
                    &session_label,
                    Some(&result.current_url),
                    completed_at,
                )?;
                db.upsert_conversation(
                    &result.conversation_id,
                    &session_id,
                    "Browser conversation",
                    Some(&result.current_url),
                    Some(&prepared.command.query),
                    Some(&result.report_text),
                    completed_at,
                )?;
                on_event(EventPayload::JobState(JobState {
                    job_id: prepared.job_id.clone(),
                    status: final_status,
                    message: Some(final_message.to_owned()),
                    conversation_id: Some(result.conversation_id.clone()),
                    updated_at: completed_at,
                    artifacts: result.artifacts.clone(),
                }))?;
                Ok(result)
            }
            Err(err) => {
                let conversation_id = conversation_id.into_inner();
                let current_url = current_url.into_inner();
                let report_text = report_text.into_inner();
                let failed_at = Utc::now();
                let message = format!("{err:#}");
                let status = if is_cancelled_message(&message) {
                    JobStatus::Cancelled
                } else {
                    JobStatus::Failed
                };
                let status_message = if status == JobStatus::Cancelled {
                    "research cancelled"
                } else {
                    "research failed"
                };
                db.insert_job(
                    &prepared.job_id,
                    status,
                    Some(&message),
                    conversation_id.as_deref(),
                    prepared.created_at,
                    failed_at,
                )?;
                db.record_job_failure(
                    &prepared.job_id,
                    failed_at,
                    &message,
                    current_url.as_deref(),
                )?;
                db.update_job_output(
                    &prepared.job_id,
                    if report_text.is_empty() {
                        None
                    } else {
                        Some(report_text.as_str())
                    },
                    current_url.as_deref(),
                    None,
                )?;
                on_event(EventPayload::JobState(JobState {
                    job_id: prepared.job_id.clone(),
                    status,
                    message: Some(status_message.to_owned()),
                    conversation_id,
                    updated_at: failed_at,
                    artifacts: vec![],
                }))?;
                Err(err)
            }
        }
    }

    pub fn status_snapshot(&self) -> shared_protocol::DaemonStatusSnapshot {
        shared_protocol::DaemonStatusSnapshot {
            pid: std::process::id(),
            started_at: self.started_at,
            state_root: self.config.paths.state_root.display().to_string(),
            ipc_endpoint: self.config.paths.ipc_endpoint.clone(),
            lock_path: self.config.paths.lock_path.display().to_string(),
            db_path: self.config.paths.db_path.display().to_string(),
            warm_mode: self.config.warm_mode(),
        }
    }

    pub fn health_snapshot(&self) -> Result<HealthCheck> {
        let browser = lock_browser(&self.browser)?.health()?;
        Ok(health_snapshot(&self.config, browser))
    }

    pub fn doctor_report(&self) -> Result<DoctorReport> {
        let db = self.open_db()?;
        let browser_snapshot = lock_browser(&self.browser)?.profile_info()?;
        let diagnostics = lock_browser(&self.browser)?.diagnostics(None).ok();
        let browser_detection = diagnostics
            .as_ref()
            .map(|value| BrowserDoctor {
                browser_detected: value.browser_detected,
                detected_browser_path: value.detected_browser_path.clone(),
                profile_accessible: value.profile_accessible,
                profile_access_error: value.profile_access_error.clone(),
            })
            .unwrap_or_else(|| BrowserDoctor {
                browser_detected: cfg!(windows) || self.config.browser_path().is_some(),
                detected_browser_path: self.config.browser_path().map(ToOwned::to_owned),
                profile_accessible: true,
                profile_access_error: None,
            });
        let latest_failure_bundle = db.latest_failure_bundle()?;
        let failure_bundle_count = db.count_failure_bundles()?;
        let health = self.health_snapshot()?;

        Ok(DoctorReport {
            health: health.clone(),
            status: self.status_snapshot(),
            config: self.config.snapshot(),
            capabilities: browser_snapshot.capability_report.clone(),
            browser: browser_snapshot.clone(),
            selector_pack: diagnostics
                .as_ref()
                .and_then(|value| value.selector_pack.clone())
                .or_else(default_selector_pack_summary),
            strategy_telemetry: diagnostics
                .as_ref()
                .map(|value| value.strategy_telemetry.clone())
                .unwrap_or_default(),
            failure_bundle_count,
            latest_failure_bundle,
            browser_detection,
            recovery_guidance: recovery_guidance(
                &self.config,
                &browser_snapshot,
                diagnostics.as_ref(),
            ),
        })
    }

    pub fn capabilities(&self) -> Result<CapabilitySnapshot> {
        Ok(CapabilitySnapshot {
            report: lock_browser(&self.browser)?.capabilities()?,
        })
    }

    pub fn config_snapshot(&self) -> ConfigSnapshot {
        self.config.snapshot()
    }

    pub fn list_sessions(&self) -> Result<SessionList> {
        self.open_db()?.list_sessions()
    }

    pub fn find_conversation(&self, conversation_id: &str) -> Result<Option<ConversationSummary>> {
        self.open_db()?.find_conversation(conversation_id)
    }

    pub fn list_jobs(&self) -> Result<JobList> {
        self.open_db()?.list_jobs()
    }

    pub fn job_detail(&self, job_id: &str) -> Result<JobDetail> {
        self.open_db()?
            .job_detail(job_id)?
            .with_context(|| format!("job {job_id} was not found"))
    }

    pub fn record_request_summary(&self, summary: &RequestSummary) -> Result<()> {
        self.open_db()?.upsert_request_summary(summary)
    }

    pub fn request_summary(&self, request_id: &str) -> Result<RequestSummary> {
        self.open_db()?
            .request_summary(request_id)?
            .with_context(|| format!("request {request_id} was not found"))
    }

    pub fn list_request_summaries(&self, limit: usize) -> Result<RequestHistory> {
        self.open_db()?.list_request_summaries(limit)
    }

    pub fn cancel_job(&self, command: CancelJobCommand) -> Result<AckEvent> {
        let db = self.open_db()?;
        let detail = db
            .job_detail(&command.job_id)?
            .with_context(|| format!("job {} was not found", command.job_id))?;
        if matches!(
            detail.status,
            JobStatus::Completed | JobStatus::Cancelled | JobStatus::Failed
        ) {
            Ok(AckEvent {
                accepted: false,
                message: format!("job {} is already finished", command.job_id),
            })
        } else {
            db.set_job_cancel_requested(&command.job_id, true)?;
            lock_cancel_registry(&self.cancel_registry)?.insert(command.job_id.clone());
            Ok(AckEvent {
                accepted: true,
                message: format!("cancellation requested for {}", command.job_id),
            })
        }
    }

    pub fn job_cancel_requested(&self, job_id: &str) -> Result<bool> {
        self.open_db()?.job_cancel_requested(job_id)
    }

    pub fn fail_job(&self, job_id: &str, message: &str) -> Result<()> {
        let db = self.open_db()?;
        let detail = db
            .job_detail(job_id)?
            .with_context(|| format!("job {job_id} was not found"))?;
        db.insert_job(
            job_id,
            JobStatus::Failed,
            Some(message),
            detail.conversation_id.as_deref(),
            detail.created_at,
            Utc::now(),
        )?;
        db.set_job_cancel_requested(job_id, false)?;
        Ok(())
    }

    pub fn mark_job_cancelled(&self, job_id: &str, message: &str) -> Result<()> {
        self.cancel_job_with_detail(job_id, None, message)
    }

    pub fn mark_job_interrupted(
        &self,
        job_id: &str,
        conversation_id: Option<&str>,
        message: &str,
    ) -> Result<()> {
        let db = self.open_db()?;
        let detail = db
            .job_detail(job_id)?
            .with_context(|| format!("job {job_id} was not found"))?;
        let interrupted_at = Utc::now();
        db.insert_job(
            job_id,
            JobStatus::Interrupted,
            Some(message),
            conversation_id.or(detail.conversation_id.as_deref()),
            detail.created_at,
            interrupted_at,
        )?;
        db.record_job_interruption(
            job_id,
            interrupted_at,
            message,
            detail.current_url.as_deref(),
        )?;
        Ok(())
    }

    pub fn mark_job_recovered(
        &self,
        job_id: &str,
        conversation_id: Option<&str>,
        message: &str,
    ) -> Result<()> {
        let db = self.open_db()?;
        let detail = db
            .job_detail(job_id)?
            .with_context(|| format!("job {job_id} was not found"))?;
        let recovered_at = Utc::now();
        db.insert_job(
            job_id,
            JobStatus::Recovered,
            Some(message),
            conversation_id.or(detail.conversation_id.as_deref()),
            detail.created_at,
            recovered_at,
        )?;
        db.record_job_recovery(job_id, recovered_at, message, detail.current_url.as_deref())?;
        Ok(())
    }

    fn cancel_job_with_detail(
        &self,
        job_id: &str,
        conversation_id: Option<&str>,
        message: &str,
    ) -> Result<()> {
        let db = self.open_db()?;
        let detail = db
            .job_detail(job_id)?
            .with_context(|| format!("job {job_id} was not found"))?;
        db.insert_job(
            job_id,
            JobStatus::Cancelled,
            Some(message),
            conversation_id.or(detail.conversation_id.as_deref()),
            detail.created_at,
            Utc::now(),
        )?;
        db.set_job_cancel_requested(job_id, false)?;
        let _ = lock_cancel_registry(&self.cancel_registry).map(|mut set| {
            set.remove(job_id);
        });
        Ok(())
    }

    pub fn list_artifacts(&self) -> Result<ArtifactList> {
        let db = self.open_db()?;
        db.prune_artifacts(ARTIFACT_RETENTION_LIMIT)?;
        db.list_artifacts()
    }

    pub fn artifact_job_ids(&self, artifact_id: &str) -> Result<Vec<String>> {
        self.open_db()?.artifact_job_ids(artifact_id)
    }

    pub fn artifact_request_ids(&self, artifact_id: &str) -> Result<Vec<String>> {
        self.open_db()?.artifact_request_ids(artifact_id)
    }

    pub fn pull_artifact(&self, command: PullArtifactCommand) -> Result<ArtifactPullResult> {
        let db = self.open_db()?;
        let artifact = if command.artifact_id == "last" {
            db.latest_artifact()?
        } else {
            db.find_artifact(&command.artifact_id)?
        }
        .with_context(|| format!("artifact {} was not found", command.artifact_id))?;

        let destination_dir = PathBuf::from(&command.destination_dir);
        fs::create_dir_all(&destination_dir)
            .with_context(|| format!("create {}", destination_dir.display()))?;
        let destination_path =
            unique_destination_path(&destination_dir, Path::new(&artifact.file_name));
        fs::copy(&artifact.storage_path, &destination_path).with_context(|| {
            format!(
                "copy managed artifact {} to {}",
                artifact.storage_path,
                destination_path.display()
            )
        })?;

        Ok(ArtifactPullResult {
            artifact,
            destination_path: destination_path.display().to_string(),
        })
    }

    pub fn browser_open(&self, request: BrowserOpenRequest) -> Result<BrowserStatusSnapshot> {
        let snapshot = lock_browser(&self.browser)?.open(request)?;
        let (session_id, session_label) = browser_session_identity(&snapshot);
        self.open_db()?.upsert_session(
            &session_id,
            &session_label,
            snapshot.current_url.as_deref(),
            Utc::now(),
        )?;
        Ok(snapshot)
    }

    pub fn browser_close(&self) -> Result<BrowserStatusSnapshot> {
        lock_browser(&self.browser)?.close()
    }

    pub fn browser_logout(&self) -> Result<BrowserStatusSnapshot> {
        let snapshot = if is_attach_primary_mode(self.config.browser_connection_mode()) {
            let current = lock_browser(&self.browser)?.health()?;
            lock_browser(&self.browser)?.open(BrowserOpenRequest {
                target_url: Some(current.target_url),
                browser_path: current.browser_path,
                headless: Some(false),
            })?
        } else {
            lock_browser(&self.browser)?.logout()?
        };
        let (session_id, session_label) = browser_session_identity(&snapshot);
        self.open_db()?.upsert_session(
            &session_id,
            &session_label,
            snapshot.current_url.as_deref(),
            Utc::now(),
        )?;
        Ok(snapshot)
    }

    pub fn browser_reset_profile(&self) -> Result<BrowserStatusSnapshot> {
        if !supports_managed_profile_reset(self.config.browser_connection_mode()) {
            anyhow::bail!(
                "browser reset-profile is only available for managed browser recovery; current mode is attach_first"
            );
        }
        lock_browser(&self.browser)?.reset_profile()
    }

    pub fn browser_profile_info(&self) -> Result<BrowserStatusSnapshot> {
        lock_browser(&self.browser)?.profile_info()
    }

    pub fn capture_failure_bundle(
        &self,
        mode: Option<shared_protocol::Mode>,
        message: &str,
        gateway_request: Option<GatewayRequestContext>,
    ) -> Result<FailureBundle> {
        let db = self.open_db()?;
        let created_at = Utc::now();
        let bundle_id = format!("bundle-{}", created_at.timestamp_millis());
        let bundle_dir = self.config.paths.crash_bundles_dir.join(&bundle_id);
        fs::create_dir_all(&bundle_dir)
            .with_context(|| format!("create {}", bundle_dir.display()))?;

        let diagnostics = lock_browser(&self.browser)
            .and_then(|mut manager| manager.diagnostics(Some(&bundle_dir)))
            .unwrap_or_else(|_| {
                let placeholder_screenshot = bundle_dir.join("failure.png");
                let _ = fs::write(&placeholder_screenshot, []);
                BrowserDiagnosticsSnapshot {
                    current_url: None,
                    capability_matrix: Default::default(),
                    strategy_attempts: vec![],
                    dom_snapshot: String::new(),
                    aria_snapshot: String::new(),
                    screenshot_path: Some(placeholder_screenshot.display().to_string()),
                    selector_pack: default_selector_pack_summary(),
                    strategy_telemetry: vec![],
                    browser_detected: cfg!(windows),
                    detected_browser_path: None,
                    profile_accessible: true,
                    profile_access_error: None,
                }
            });

        let dom_snapshot_path = bundle_dir.join("dom.json");
        fs::write(&dom_snapshot_path, diagnostics.dom_snapshot.as_bytes())
            .with_context(|| format!("write {}", dom_snapshot_path.display()))?;
        let aria_snapshot_path = bundle_dir.join("aria.json");
        fs::write(&aria_snapshot_path, diagnostics.aria_snapshot.as_bytes())
            .with_context(|| format!("write {}", aria_snapshot_path.display()))?;

        let bundle = FailureBundle {
            bundle_id,
            error_code: error_code_for_message(message),
            message: message.to_owned(),
            created_at,
            url: diagnostics.current_url,
            mode,
            screenshot_path: diagnostics.screenshot_path,
            dom_snapshot_path: Some(dom_snapshot_path.display().to_string()),
            aria_snapshot_path: Some(aria_snapshot_path.display().to_string()),
            logs_path: Some(self.config.paths.log_path.display().to_string()),
            strategy_attempts: diagnostics.strategy_attempts,
            capability_matrix: diagnostics.capability_matrix,
            gateway_request,
        };
        let bundle_json_path = bundle_dir.join("bundle.json");
        fs::write(&bundle_json_path, serde_json::to_vec_pretty(&bundle)?)
            .with_context(|| format!("write {}", bundle_json_path.display()))?;
        db.insert_failure_bundle(&bundle)?;
        Ok(bundle)
    }

    fn open_db(&self) -> Result<Database> {
        Database::open(&self.config.paths.db_path)
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_request_lifecycle_update<F>(
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

pub fn error_code_for_message(message: &str) -> ErrorCode {
    if message.contains("profile locked") || message.contains("SingletonLock") {
        ErrorCode::ProfileLocked
    } else if message.contains("auth required") {
        ErrorCode::AuthRequired
    } else if message.contains("browser launch failed")
        || message.contains("spawn browser adapter")
        || message.contains("supported browser executable")
    {
        ErrorCode::BrowserLaunch
    } else if message.contains("capability missing") {
        ErrorCode::CapabilityMissing
    } else if message.contains("research job failed")
        || message.contains("research failed")
        || message.contains("research cancelled")
    {
        ErrorCode::ResearchJobFailed
    } else if message.contains("upload failed") {
        ErrorCode::UploadFailed
    } else if message.contains("download failed") {
        ErrorCode::DownloadFailed
    } else if message.contains("ui drift") {
        ErrorCode::UiDrift
    } else if message.contains("timed out") {
        ErrorCode::ActionTimeout
    } else {
        ErrorCode::Transport
    }
}

fn lock_browser<'a>(
    browser: &'a Arc<Mutex<BrowserManager>>,
) -> Result<MutexGuard<'a, BrowserManager>> {
    browser
        .lock()
        .map_err(|_| anyhow!("browser manager lock poisoned"))
}

fn lock_cancel_registry<'a>(
    registry: &'a Arc<Mutex<HashSet<String>>>,
) -> Result<MutexGuard<'a, HashSet<String>>> {
    registry
        .lock()
        .map_err(|_| anyhow!("cancel registry lock poisoned"))
}

fn is_cancelled_message(message: &str) -> bool {
    message.contains("cancelled")
}

fn health_snapshot(config: &AppConfig, browser: BrowserStatusSnapshot) -> HealthCheck {
    let status = if !browser.running || !browser.extension_connected || !browser.authenticated {
        HealthStatus::Degraded
    } else {
        HealthStatus::Ok
    };
    HealthCheck {
        status,
        checked_at: Utc::now(),
        browser_connected: browser.running,
        extension_connected: browser.extension_connected,
        authenticated: browser.authenticated,
        details: Some(if config.warm_mode() {
            format!(
                "control plane ready; warm mode enabled; attach_mode={:?}; browser_running={}; extension_connected={}",
                browser.attach_mode, browser.running, browser.extension_connected
            )
        } else {
            browser_health_details(&browser, config)
        }),
    }
}

fn recovery_guidance(
    config: &AppConfig,
    browser: &BrowserStatusSnapshot,
    diagnostics: Option<&BrowserDiagnosticsSnapshot>,
) -> Vec<String> {
    let mut guidance = Vec::new();
    let mut attach_problem = false;
    let debug_endpoint = browser
        .debug_endpoint
        .clone()
        .unwrap_or_else(|| config.browser_debug_endpoint());
    if config.safe_mode() {
        guidance.push(
            "safe mode is enabled; ask and research are disabled until config.toml is updated"
                .to_owned(),
        );
    }
    if is_attach_primary_mode(browser.attach_mode) {
        if !browser.running {
            attach_problem = true;
            guidance.push(format!(
                "attach endpoint is not reachable at {debug_endpoint}; fully close Chrome or Edge, run `gpt browser start-debug`, then rerun `gpt browser attach`"
            ));
            guidance.push(
                "run `gpt browser connection-info` and confirm the configured debug endpoint, target URL, and browser path before switching modes"
                    .to_owned(),
            );
        }
        if browser.extension_required && !browser.extension_connected {
            attach_problem = true;
            guidance.push(
                "the Tablinq extension is missing or disconnected in the attached browser profile; confirm it is installed, enabled, and running in the same browser profile Tablinq is attaching to"
                    .to_owned(),
            );
        }
        if current_url_requires_login_or_challenge(browser.current_url.as_deref()) {
            attach_problem = true;
            guidance.push(
                "the attached ChatGPT tab is blocked on sign-in or a challenge page; complete that page in your normal browser, then rerun `gpt browser attach`"
                    .to_owned(),
            );
        } else if !browser.authenticated {
            attach_problem = true;
            guidance.push(
                "the attached ChatGPT tab is signed out; sign in to ChatGPT in your normal browser, then rerun `gpt browser attach`"
                    .to_owned(),
            );
        }
        if !browser.target_tab_found {
            attach_problem = true;
            guidance.push(
                "target tab not found; run `gpt browser open` to create or focus the configured ChatGPT tab after the browser is signed in"
                    .to_owned(),
            );
        }
        if browser.running
            && (!browser.target_tab_found
                || !browser.authenticated
                || (browser.extension_required && !browser.extension_connected))
        {
            guidance.push(
                "if multiple ChatGPT tabs are open, keep only the signed-in tab with the working Tablinq extension, then rerun `gpt browser attach`"
                    .to_owned(),
            );
        }
        if attach_problem {
            guidance.push(
                "only if your machine or browser profile cannot expose the local debug endpoint, set `browser_connection_mode = \"managed_only\"` in config.toml and use `gpt browser open` as explicit fallback"
                    .to_owned(),
            );
        }
        if browser.running
            && browser.target_tab_found
            && browser.authenticated
            && (!browser.extension_required || browser.extension_connected)
        {
            guidance.push("attached browser session is ready".to_owned());
        }
    } else {
        if !browser.running {
            guidance.push(
                "managed browser fallback is not running; use `gpt browser open` to start the managed browser tab"
                    .to_owned(),
            );
        } else {
            guidance.push(
                "daemon can recreate the managed fallback tab and relaunch the browser adapter when the page crashes or closes"
                    .to_owned(),
            );
        }
        if !browser.authenticated {
            guidance.push(
                "managed browser fallback is signed out; open the managed browser and sign in before running ask or research"
                    .to_owned(),
            );
        }
        if !browser.extension_connected {
            guidance.push(
                "extension probe is disconnected in managed fallback mode; reload the page or relaunch the browser to refresh capabilities"
                    .to_owned(),
            );
        }
    }
    if let Some(diagnostics) = diagnostics {
        if !diagnostics.browser_detected {
            guidance.push(
                "no supported browser was detected; set an explicit browser path in config.toml"
                    .to_owned(),
            );
        }
        if !diagnostics.profile_accessible && supports_managed_profile_reset(browser.attach_mode) {
            guidance.push(
                "browser profile is not writable; close other managed browser instances or run `gpt browser reset-profile`"
                    .to_owned(),
            );
        }
    }
    guidance
}

fn current_url_requires_login_or_challenge(current_url: Option<&str>) -> bool {
    let Some(current_url) = current_url else {
        return false;
    };
    let lowered = current_url.to_ascii_lowercase();
    ["challenge", "captcha", "login", "signin", "auth", "cf_chl"]
        .iter()
        .any(|marker| lowered.contains(marker))
}

fn browser_health_details(browser: &BrowserStatusSnapshot, config: &AppConfig) -> String {
    if !browser.running {
        return if is_attach_primary_mode(browser.attach_mode) {
            format!(
                "control plane ready; attach-first browser not connected; debug_endpoint={}",
                browser
                    .debug_endpoint
                    .clone()
                    .unwrap_or_else(|| config.browser_debug_endpoint())
            )
        } else {
            "control plane ready; managed browser fallback not started yet".to_owned()
        };
    }

    if is_attach_primary_mode(browser.attach_mode) {
        if !browser.target_tab_found {
            return "control plane ready; attached browser is connected but the target tab was not found"
                .to_owned();
        }
        return match browser.current_url.as_deref() {
            Some(url) => format!(
                "control plane ready; attached tab at {url}; authenticated={}; extension_connected={}",
                browser.authenticated, browser.extension_connected
            ),
            None => format!(
                "control plane ready; attached browser is connected; authenticated={}; extension_connected={}",
                browser.authenticated, browser.extension_connected
            ),
        };
    }

    match browser.current_url.as_deref() {
        Some(url) => format!(
            "control plane ready; managed fallback tab at {url}; extension_connected={}",
            browser.extension_connected
        ),
        None => "control plane ready; managed browser fallback is connected".to_owned(),
    }
}

fn is_attach_primary_mode(mode: BrowserConnectionMode) -> bool {
    matches!(
        mode,
        BrowserConnectionMode::AttachFirst | BrowserConnectionMode::ManagedFallback
    )
}

fn supports_managed_profile_reset(mode: BrowserConnectionMode) -> bool {
    matches!(
        mode,
        BrowserConnectionMode::ManagedFallback | BrowserConnectionMode::ManagedOnly
    )
}

fn browser_session_identity(snapshot: &BrowserStatusSnapshot) -> (String, String) {
    if snapshot.cdp_attached {
        if let Some(tab_id) = snapshot
            .target_tab_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            let short_id = &tab_id[..tab_id.len().min(8)];
            return (
                format!("browser-tab-{tab_id}"),
                format!("Browser tab {short_id}"),
            );
        }
    }

    ("browser-primary".to_owned(), "Browser session".to_owned())
}

fn default_selector_pack_summary() -> Option<SelectorPackSummary> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("selector-packs")
        .join("v1")
        .join("chatgpt-default.json");
    let raw = fs::read_to_string(&path).ok()?;
    let parsed: shared_protocol::SelectorPack = serde_json::from_str(&raw).ok()?;
    Some(SelectorPackSummary {
        pack_id: parsed.pack_id,
        version: format!("v{}", parsed.format_version),
        updated_at: Some(parsed.updated_at),
        path: Some(path.display().to_string()),
    })
}

#[allow(clippy::too_many_arguments)]
fn apply_research_update<F>(
    db: &Database,
    job_id: &str,
    created_at: chrono::DateTime<Utc>,
    conversation_id: &mut Option<String>,
    current_url: &mut Option<String>,
    report_text: &mut String,
    next_seq: &mut u64,
    update: ResearchUpdate,
    on_event: &mut F,
) -> Result<()>
where
    F: FnMut(EventPayload) -> Result<()>,
{
    let updated_at = Utc::now();
    if let Some(value) = update.conversation_id.clone() {
        *conversation_id = Some(value);
    }
    if let Some(value) = update.current_url.clone() {
        *current_url = Some(value);
    }

    db.insert_job(
        job_id,
        update.status,
        update.message.as_deref(),
        update
            .conversation_id
            .as_deref()
            .or(conversation_id.as_deref()),
        created_at,
        updated_at,
    )?;

    if let Some(chunk_text) = update.chunk_text.clone() {
        if matches!(
            update.chunk_kind.unwrap_or(StreamChunkKind::Delta),
            StreamChunkKind::Delta
        ) {
            report_text.push_str(&chunk_text);
        }
        on_event(EventPayload::StreamChunk(StreamChunk {
            job_id: job_id.to_owned(),
            conversation_id: update.conversation_id.clone().or(conversation_id.clone()),
            message_id: update.message_id.clone(),
            seq: *next_seq,
            kind: update.chunk_kind.unwrap_or(StreamChunkKind::Delta),
            text: chunk_text,
        }))?;
        *next_seq += 1;
    }

    db.update_job_output(
        job_id,
        if report_text.is_empty() {
            None
        } else {
            Some(report_text.as_str())
        },
        current_url.as_deref(),
        None,
    )?;
    on_event(EventPayload::JobState(JobState {
        job_id: job_id.to_owned(),
        status: update.status,
        message: update.message,
        conversation_id: update.conversation_id.or(conversation_id.clone()),
        updated_at,
        artifacts: vec![],
    }))
}

fn maybe_write_research_output(
    command: &ResearchCommand,
    job_id: &str,
    report_text: &str,
) -> Result<Option<String>> {
    let Some(output_dir) = command.output_dir.as_deref() else {
        return Ok(None);
    };

    let output_dir = PathBuf::from(output_dir);
    fs::create_dir_all(&output_dir).with_context(|| format!("create {}", output_dir.display()))?;
    let destination = unique_destination_path(
        &output_dir,
        Path::new(&format!("{}-report.md", sanitize_file_stem(job_id))),
    );
    fs::write(&destination, report_text)
        .with_context(|| format!("write {}", destination.display()))?;
    Ok(Some(destination.display().to_string()))
}

fn sanitize_file_stem(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn unique_destination_path(destination_dir: &Path, file_name: &Path) -> PathBuf {
    let candidate = destination_dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let stem = file_name
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("artifact");
    let extension = file_name.extension().and_then(|value| value.to_str());

    for index in 2.. {
        let name = match extension {
            Some(extension) => format!("{stem}-{index}.{extension}"),
            None => format!("{stem}-{index}"),
        };
        let candidate = destination_dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("destination path generation should always return");
}

#[cfg(test)]
mod tests {
    use super::{
        browser_session_identity, current_url_requires_login_or_challenge, recovery_guidance,
    };
    use crate::AppConfig;
    use shared_protocol::{AuthSource, BrowserConnectionMode, BrowserOwner, BrowserStatusSnapshot};

    #[test]
    fn attach_session_identity_uses_target_tab_id() {
        let snapshot = BrowserStatusSnapshot {
            running: true,
            browser_kind: "chrome".to_owned(),
            profile_dir: "profile".to_owned(),
            downloads_dir: "downloads".to_owned(),
            target_url: "https://chatgpt.com/".to_owned(),
            current_url: Some("https://chatgpt.com/?conversation=conv-1".to_owned()),
            page_title: None,
            page_ready: true,
            authenticated: true,
            reload_count: 0,
            screenshot_path: None,
            browser_path: None,
            extension_connected: true,
            capability_report: None,
            attach_mode: BrowserConnectionMode::AttachFirst,
            browser_owner: BrowserOwner::External,
            cdp_attached: true,
            target_tab_found: true,
            target_tab_id: Some("abcdef1234567890".to_owned()),
            target_workspace_key: Some("https://chatgpt.com/".to_owned()),
            debug_endpoint: Some("http://127.0.0.1:9222".to_owned()),
            auth_source: AuthSource::Page,
            extension_required: true,
            last_capability_report_at: None,
        };

        let (session_id, label) = browser_session_identity(&snapshot);

        assert_eq!(session_id, "browser-tab-abcdef1234567890");
        assert_eq!(label, "Browser tab abcdef12");
    }

    #[test]
    fn attach_recovery_guidance_prioritizes_endpoint_extension_auth_tab_and_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        let browser = BrowserStatusSnapshot {
            running: false,
            browser_kind: "chrome".to_owned(),
            profile_dir: "profile".to_owned(),
            downloads_dir: "downloads".to_owned(),
            target_url: "https://chatgpt.com/".to_owned(),
            current_url: Some("https://chatgpt.com/auth/login".to_owned()),
            page_title: None,
            page_ready: false,
            authenticated: false,
            reload_count: 0,
            screenshot_path: None,
            browser_path: None,
            extension_connected: false,
            capability_report: None,
            attach_mode: BrowserConnectionMode::AttachFirst,
            browser_owner: BrowserOwner::External,
            cdp_attached: false,
            target_tab_found: false,
            target_tab_id: None,
            target_workspace_key: None,
            debug_endpoint: Some("http://127.0.0.1:9222".to_owned()),
            auth_source: AuthSource::Unknown,
            extension_required: true,
            last_capability_report_at: None,
        };

        let guidance = recovery_guidance(&config, &browser, None);

        assert!(guidance[0].contains("attach endpoint is not reachable"));
        assert!(guidance[1].contains("gpt browser connection-info"));
        assert!(guidance[2].contains("Tablinq extension is missing or disconnected"));
        assert!(guidance[3].contains("sign-in or a challenge page"));
        assert!(guidance[4].contains("target tab not found"));
        assert!(
            guidance
                .last()
                .unwrap()
                .contains("browser_connection_mode = \"managed_only\"")
        );
    }

    #[test]
    fn current_url_requires_login_or_challenge_detects_cloudflare_and_login_pages() {
        assert!(current_url_requires_login_or_challenge(Some(
            "https://chatgpt.com/auth/login"
        )));
        assert!(current_url_requires_login_or_challenge(Some(
            "https://chatgpt.com/?cf_chl_tk=token"
        )));
        assert!(!current_url_requires_login_or_challenge(Some(
            "https://chatgpt.com/?conversation=conv-1"
        )));
    }
}
