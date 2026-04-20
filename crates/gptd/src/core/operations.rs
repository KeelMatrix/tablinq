use super::support::{
    browser_busy_message, browser_session_identity, default_selector_pack_summary, health_snapshot,
    is_attach_primary_mode, lock_browser, lock_cancel_registry, recovery_guidance,
    supports_managed_profile_reset, try_lock_browser,
};
use super::{ARTIFACT_RETENTION_LIMIT, CoreService, error_code_for_message};
use crate::browser::BrowserDiagnosticsSnapshot;
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use shared_protocol::{
    AckEvent, ArtifactList, ArtifactPullResult, BrowserDoctor, BrowserOpenRequest,
    BrowserStatusSnapshot, CancelJobCommand, CapabilitySnapshot, ConfigSnapshot,
    ConversationSummary, DoctorReport, FailureBundle, GatewayRequestContext, JobDetail, JobList,
    JobStatus, PullArtifactCommand, RequestHistory, RequestSummary, SessionList,
};
use std::fs;
use std::path::{Path, PathBuf};

impl CoreService {
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

    pub fn health_snapshot(&self) -> Result<shared_protocol::HealthCheck> {
        let Some(mut browser) = try_lock_browser(&self.browser)? else {
            return Ok(super::support::busy_health_snapshot());
        };
        Ok(health_snapshot(&self.config, browser.health()?))
    }

    pub fn doctor_report(&self) -> Result<DoctorReport> {
        let db = self.open_db()?;
        let mut browser = try_lock_browser(&self.browser)?
            .ok_or_else(|| anyhow!(browser_busy_message()))?;
        let browser_snapshot = browser.profile_info()?;
        let diagnostics = browser.diagnostics(None).ok();
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
        let health = health_snapshot(&self.config, browser_snapshot.clone());

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
            report: try_lock_browser(&self.browser)?
                .ok_or_else(|| anyhow!(browser_busy_message()))?
                .capabilities()?,
        })
    }

    pub fn config_snapshot(&self) -> ConfigSnapshot {
        self.config.snapshot()
    }

    pub fn list_sessions(&self) -> Result<SessionList> {
        self.open_db()?.list_sessions()
    }

    pub fn find_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationSummary>> {
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

    pub(super) fn cancel_job_with_detail(
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
            super::support::unique_destination_path(&destination_dir, Path::new(&artifact.file_name));
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
        let snapshot = try_lock_browser(&self.browser)?
            .ok_or_else(|| anyhow!(browser_busy_message()))?
            .open(request)?;
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
        try_lock_browser(&self.browser)?
            .ok_or_else(|| anyhow!(browser_busy_message()))?
            .close()
    }

    pub fn browser_logout(&self) -> Result<BrowserStatusSnapshot> {
        let snapshot = if is_attach_primary_mode(self.config.browser_connection_mode()) {
            let mut browser = try_lock_browser(&self.browser)?
                .ok_or_else(|| anyhow!(browser_busy_message()))?;
            let current = browser.health()?;
            browser.open(BrowserOpenRequest {
                target_url: Some(current.target_url),
                browser_path: current.browser_path,
                headless: Some(false),
            })?
        } else {
            try_lock_browser(&self.browser)?
                .ok_or_else(|| anyhow!(browser_busy_message()))?
                .logout()?
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
        try_lock_browser(&self.browser)?
            .ok_or_else(|| anyhow!(browser_busy_message()))?
            .reset_profile()
    }

    pub fn browser_profile_info(&self) -> Result<BrowserStatusSnapshot> {
        try_lock_browser(&self.browser)?
            .ok_or_else(|| anyhow!(browser_busy_message()))?
            .profile_info()
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
}
