use crate::browser::{BrowserDiagnosticsSnapshot, BrowserManager, ResearchUpdate};
use crate::config::AppConfig;
use crate::db::Database;
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use shared_protocol::{
    AckEvent, ArtifactList, ArtifactPullResult, AskCommand, AskResult, BrowserDoctor,
    BrowserOpenRequest, BrowserStatusSnapshot, CancelJobCommand, CapabilitySnapshot,
    ConfigSnapshot, DoctorReport, ErrorCode, EventPayload, FailureBundle, GatewayRequestContext,
    HealthCheck, HealthStatus, JobDetail, JobKind, JobList, JobState, JobStatus,
    PullArtifactCommand, ResearchCommand, ResearchResult, SelectorPackSummary, SessionList,
    StreamChunk, StreamChunkKind,
};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

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
            conversation_id: None,
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
        let conversation_id = match command.conversation_id.clone() {
            Some(value) => Some(value),
            None => db.latest_conversation_id()?,
        };
        let mut effective = command.clone();
        effective.conversation_id = conversation_id.clone();

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
            self.fail_job(&prepared.job_id, message)?;
            on_event(EventPayload::JobState(JobState {
                job_id: prepared.job_id.clone(),
                status: JobStatus::Failed,
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

        let outcome = lock_browser(&self.browser)?.ask(
            &prepared.job_id,
            &prepared.command,
            |event| on_event(event),
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
                let completed_at = result.completed_at;
                for artifact in &result.artifacts {
                    db.insert_artifact(artifact, completed_at)?;
                    db.link_job_artifact(&prepared.job_id, &artifact.artifact_id)?;
                    on_event(EventPayload::ArtifactReady(artifact.clone()))?;
                }
                db.insert_job(
                    &prepared.job_id,
                    JobStatus::Completed,
                    Some("ask completed"),
                    Some(&result.conversation_id),
                    prepared.created_at,
                    completed_at,
                )?;
                db.upsert_session(
                    "browser-primary",
                    "Managed browser",
                    Some(&result.current_url),
                    completed_at,
                )?;
                db.upsert_conversation(
                    &result.conversation_id,
                    "browser-primary",
                    "Managed conversation",
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
                    status: JobStatus::Completed,
                    message: Some("ask completed".to_owned()),
                    conversation_id: Some(result.conversation_id.clone()),
                    updated_at: completed_at,
                    artifacts: result.artifacts.clone(),
                }))?;
                Ok(result)
            }
            Err(err) => {
                let failed_at = Utc::now();
                let message = format!("{err:#}");
                db.insert_job(
                    &prepared.job_id,
                    JobStatus::Failed,
                    Some(&message),
                    prepared.command.conversation_id.as_deref(),
                    prepared.created_at,
                    failed_at,
                )?;
                db.update_job_output(&prepared.job_id, None, None, None)?;
                on_event(EventPayload::JobState(JobState {
                    job_id: prepared.job_id.clone(),
                    status: JobStatus::Failed,
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
        db.insert_job(
            &job_id,
            JobStatus::Queued,
            Some("queued research job"),
            None,
            created_at,
            created_at,
        )?;
        db.upsert_job_detail(&job_id, JobKind::Research, Some(&command.query), None)?;

        Ok(PreparedResearchJob {
            job_id,
            command,
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
            self.fail_job(&prepared.job_id, message)?;
            on_event(EventPayload::JobState(JobState {
                job_id: prepared.job_id.clone(),
                status: JobStatus::Failed,
                message: Some(message.to_owned()),
                conversation_id: None,
                updated_at: Utc::now(),
                artifacts: vec![],
            }))?;
            anyhow::bail!(message);
        }

        let mut conversation_id = None;
        let mut current_url = None;
        let mut report_text = String::new();
        let mut next_seq = 0u64;

        let outcome = lock_browser(&self.browser)?.research(
            &prepared.job_id,
            &prepared.command,
            |update| {
                apply_research_update(
                    &db,
                    &prepared.job_id,
                    prepared.created_at,
                    &mut conversation_id,
                    &mut current_url,
                    &mut report_text,
                    &mut next_seq,
                    update,
                    &mut on_event,
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
                db.insert_job(
                    &prepared.job_id,
                    JobStatus::Completed,
                    Some("research completed"),
                    Some(&result.conversation_id),
                    prepared.created_at,
                    completed_at,
                )?;
                db.update_job_output(
                    &prepared.job_id,
                    Some(&result.report_text),
                    Some(&result.current_url),
                    output_path.as_deref(),
                )?;
                db.upsert_session(
                    "browser-primary",
                    "Managed browser",
                    Some(&result.current_url),
                    completed_at,
                )?;
                db.upsert_conversation(
                    &result.conversation_id,
                    "browser-primary",
                    "Managed conversation",
                    Some(&result.current_url),
                    Some(&prepared.command.query),
                    Some(&result.report_text),
                    completed_at,
                )?;
                on_event(EventPayload::JobState(JobState {
                    job_id: prepared.job_id.clone(),
                    status: JobStatus::Completed,
                    message: Some("research completed".to_owned()),
                    conversation_id: Some(result.conversation_id.clone()),
                    updated_at: completed_at,
                    artifacts: result.artifacts.clone(),
                }))?;
                Ok(result)
            }
            Err(err) => {
                let failed_at = Utc::now();
                let message = format!("{err:#}");
                let status_message = if message.contains("cancelled") {
                    "research cancelled"
                } else {
                    "research failed"
                };
                db.insert_job(
                    &prepared.job_id,
                    JobStatus::Failed,
                    Some(&message),
                    conversation_id.as_deref(),
                    prepared.created_at,
                    failed_at,
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
                    status: JobStatus::Failed,
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
            browser: browser_snapshot,
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
            recovery_guidance: recovery_guidance(&self.config, health, diagnostics.as_ref()),
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

    pub fn list_jobs(&self) -> Result<JobList> {
        self.open_db()?.list_jobs()
    }

    pub fn job_detail(&self, job_id: &str) -> Result<JobDetail> {
        self.open_db()?
            .job_detail(job_id)?
            .with_context(|| format!("job {job_id} was not found"))
    }

    pub fn cancel_job(&self, command: CancelJobCommand) -> Result<AckEvent> {
        let db = self.open_db()?;
        let detail = db
            .job_detail(&command.job_id)?
            .with_context(|| format!("job {} was not found", command.job_id))?;
        if matches!(detail.status, JobStatus::Completed | JobStatus::Failed) {
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

    pub fn list_artifacts(&self) -> Result<ArtifactList> {
        self.open_db()?.list_artifacts()
    }

    pub fn artifact_job_ids(&self, artifact_id: &str) -> Result<Vec<String>> {
        self.open_db()?.artifact_job_ids(artifact_id)
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
        self.open_db()?.upsert_session(
            "browser-primary",
            "Managed browser",
            snapshot.current_url.as_deref(),
            Utc::now(),
        )?;
        Ok(snapshot)
    }

    pub fn browser_close(&self) -> Result<BrowserStatusSnapshot> {
        lock_browser(&self.browser)?.close()
    }

    pub fn browser_logout(&self) -> Result<BrowserStatusSnapshot> {
        let snapshot = lock_browser(&self.browser)?.logout()?;
        self.open_db()?.upsert_session(
            "browser-primary",
            "Managed browser",
            snapshot.current_url.as_deref(),
            Utc::now(),
        )?;
        Ok(snapshot)
    }

    pub fn browser_reset_profile(&self) -> Result<BrowserStatusSnapshot> {
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

fn health_snapshot(config: &AppConfig, browser: BrowserStatusSnapshot) -> HealthCheck {
    let status = if !browser.running {
        HealthStatus::Degraded
    } else if !browser.extension_connected || !browser.authenticated {
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
                "control plane ready; warm mode enabled; browser_running={}; extension_connected={}",
                browser.running, browser.extension_connected
            )
        } else {
            match browser.current_url {
                Some(url) => format!(
                    "control plane ready; managed tab at {url}; extension_connected={}",
                    browser.extension_connected
                ),
                None => "control plane ready; browser runtime not started yet".to_owned(),
            }
        }),
    }
}

fn recovery_guidance(
    config: &AppConfig,
    health: HealthCheck,
    diagnostics: Option<&BrowserDiagnosticsSnapshot>,
) -> Vec<String> {
    let mut guidance = Vec::new();
    if config.safe_mode() {
        guidance.push(
            "safe mode is enabled; ask and research are disabled until config.toml is updated"
                .to_owned(),
        );
    }
    if !health.browser_connected {
        guidance.push(
            "browser is not running; use `gpt browser open` to launch or recreate the managed tab"
                .to_owned(),
        );
    } else {
        guidance.push("daemon can recreate the managed tab and relaunch the browser adapter when the page crashes or closes".to_owned());
    }
    if !health.authenticated {
        guidance.push("authentication is required; open the managed browser and sign in before running ask or research".to_owned());
    }
    if !health.extension_connected {
        guidance.push("extension probe is disconnected; reload the managed page or relaunch the browser to refresh capabilities".to_owned());
    }
    if let Some(diagnostics) = diagnostics {
        if !diagnostics.profile_accessible {
            guidance.push("browser profile is not writable; close other managed browser instances or reset the profile directory".to_owned());
        }
        if !diagnostics.browser_detected {
            guidance.push(
                "no supported browser was detected; set an explicit browser path in config.toml"
                    .to_owned(),
            );
        }
    }
    guidance
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
