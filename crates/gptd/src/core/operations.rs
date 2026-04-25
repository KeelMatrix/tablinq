use super::support::{
    browser_busy_message, browser_session_identity, default_selector_pack_summary, health_snapshot,
    is_attach_primary_mode, lock_browser, lock_cancel_registry, recovery_guidance,
    resolve_conversation_target_url, supports_managed_profile_reset, try_lock_browser,
};
use super::{ARTIFACT_RETENTION_LIMIT, CoreService, error_code_for_message};
use crate::browser::BrowserDiagnosticsSnapshot;
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use shared_protocol::{
    AckEvent, ArtifactList, ArtifactMetadata, ArtifactPullResult, ArtifactSummary, BrowserDoctor,
    BrowserOpenRequest, BrowserStatusSnapshot, CancelJobCommand, CapabilitySnapshot,
    ConfigSnapshot, ConversationList, ConversationSelectCommand, ConversationSelection,
    ConversationSummary, DoctorReport, FailureBundle, GatewayRequestContext, JobDetail, JobList,
    JobStatus, PullArtifactCommand, RequestHistory, RequestListPage, RequestSummary,
    SessionConversationList, SessionList,
};
use std::fs;
use std::path::{Path, PathBuf};

const EVIDENCE_ARTIFACT_CONTENT_TYPE: &str = "application/tablinq-browser-evidence+json";
#[derive(Serialize)]
struct BrowserEvidencePayload {
    request_id: String,
    job_id: String,
    captured_at: chrono::DateTime<Utc>,
    current_url: Option<String>,
    screenshot_path: Option<String>,
    dom_snapshot: String,
}

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
        let mut browser =
            try_lock_browser(&self.browser)?.ok_or_else(|| anyhow!(browser_busy_message()))?;
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

    pub fn find_conversation(&self, conversation_id: &str) -> Result<Option<ConversationSummary>> {
        self.open_db()?.find_conversation(conversation_id)
    }

    pub fn list_conversations(&self) -> Result<ConversationList> {
        self.open_db()?.list_conversations()
    }

    pub fn list_session_conversations(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionConversationList>> {
        let db = self.open_db()?;
        let Some(session) = db.find_session_info(session_id)? else {
            return Ok(None);
        };
        Ok(Some(SessionConversationList {
            session,
            conversations: db.list_session_conversations(session_id)?,
        }))
    }

    pub fn select_conversation(
        &self,
        command: ConversationSelectCommand,
    ) -> Result<ConversationSelection> {
        command
            .validate()
            .context("validate conversation select command")?;

        let target_conversation = if let Some(conversation_id) = command.conversation_id.as_deref()
        {
            Some(
                self.open_db()?
                    .find_conversation(conversation_id)?
                    .with_context(|| format!("conversation {conversation_id} was not found"))?,
            )
        } else {
            None
        };

        let target_url = if let Some(conversation) = target_conversation.as_ref() {
            conversation
                .tab_url
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    resolve_conversation_target_url(
                        self.config.browser_target_url(),
                        &conversation.conversation_id,
                    )
                })
        } else {
            self.config.browser_target_url().to_owned()
        };

        let snapshot = self.browser_open(BrowserOpenRequest {
            target_url: Some(target_url),
            browser_path: None,
            headless: None,
        })?;
        let (session_id, _) = browser_session_identity(&snapshot);

        Ok(ConversationSelection {
            conversation_id: target_conversation.map(|conversation| conversation.conversation_id),
            session_id: Some(session_id),
            current_url: snapshot.current_url.clone(),
            browser: snapshot,
        })
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

    pub fn list_request_page(
        &self,
        status: Option<JobStatus>,
        limit: usize,
        offset: usize,
    ) -> Result<RequestListPage> {
        self.open_db()?.list_request_page(status, limit, offset)
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

    pub fn latest_request_evidence(&self, request_id: &str) -> Result<ArtifactSummary> {
        self.open_db()?
            .latest_request_artifact_by_content_type(request_id, EVIDENCE_ARTIFACT_CONTENT_TYPE)?
            .with_context(|| format!("evidence for request {request_id} was not found"))
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
        let destination_path = super::support::unique_destination_path(
            &destination_dir,
            Path::new(&artifact.file_name),
        );
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
            let mut browser =
                try_lock_browser(&self.browser)?.ok_or_else(|| anyhow!(browser_busy_message()))?;
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

    pub fn capture_browser_evidence(&self, request_id: &str) -> Result<ArtifactMetadata> {
        let db = self.open_db()?;
        let request = self.request_summary(request_id)?;
        let job = self.job_detail(&request.job_id)?;
        let created_at = Utc::now();
        let evidence_dir = self.config.paths.crash_bundles_dir.join(format!(
            "evidence-{request_id}-{}",
            created_at.timestamp_millis()
        ));
        fs::create_dir_all(&evidence_dir)
            .with_context(|| format!("create {}", evidence_dir.display()))?;

        let diagnostics = capture_diagnostics_snapshot(&self.browser, &evidence_dir);
        let payload = BrowserEvidencePayload {
            request_id: request.request_id.clone(),
            job_id: request.job_id.clone(),
            captured_at: created_at,
            current_url: diagnostics.current_url.clone(),
            screenshot_path: diagnostics.screenshot_path.clone(),
            dom_snapshot: diagnostics.dom_snapshot,
        };
        let artifact_path = evidence_dir.join("evidence.json");
        let payload_bytes = serde_json::to_vec_pretty(&payload)?;
        fs::write(&artifact_path, &payload_bytes)
            .with_context(|| format!("write {}", artifact_path.display()))?;

        let artifact = ArtifactMetadata {
            artifact_id: format!("artifact-evidence-{}", created_at.timestamp_millis()),
            conversation_id: job.conversation_id,
            message_id: None,
            file_name: format!("request-evidence-{request_id}.json"),
            content_type: EVIDENCE_ARTIFACT_CONTENT_TYPE.to_owned(),
            size_bytes: payload_bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&payload_bytes)),
            storage_path: artifact_path.display().to_string(),
        };
        db.insert_artifact(&artifact, created_at)?;
        db.link_job_artifact(&request.job_id, &artifact.artifact_id)?;
        db.prune_artifacts(ARTIFACT_RETENTION_LIMIT)?;
        Ok(artifact)
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

        let diagnostics = capture_diagnostics_snapshot(&self.browser, &bundle_dir);

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

fn capture_diagnostics_snapshot(
    browser: &std::sync::Arc<std::sync::Mutex<crate::browser::BrowserManager>>,
    output_dir: &Path,
) -> BrowserDiagnosticsSnapshot {
    lock_browser(browser)
        .and_then(|mut manager| manager.diagnostics(Some(output_dir)))
        .unwrap_or_else(|_| placeholder_diagnostics_snapshot(output_dir))
}

fn placeholder_diagnostics_snapshot(output_dir: &Path) -> BrowserDiagnosticsSnapshot {
    let placeholder_screenshot = output_dir.join("failure.png");
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppConfig, Database};
    use chrono::{TimeZone, Utc};
    use shared_protocol::{
        ArtifactMetadata, ConversationMessageExactness, ConversationMessageRole,
        ConversationMessageSourceKind, ConversationRecoveryManifest, Effort, ErrorCode, JobKind,
        JobStatus, Mode, ModelClass, RecoveryManifestFile, RecoveryManifestFileKind,
        RecoveryManifestTurn, RecoveryPacket, RecoveryStrategy, RecoveryTrace,
        RequestExecutionKind, RequestUsage,
    };
    use tempfile::{TempDir, tempdir};

    fn fixed_time() -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 0).unwrap()
    }

    fn make_core() -> (TempDir, AppConfig, CoreService) {
        let dir = tempdir().unwrap();
        let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        let core = CoreService::new(config.clone()).unwrap();
        (dir, config, core)
    }

    fn seed_conversation_fixtures(config: &AppConfig) {
        let db = Database::open(&config.paths.db_path).unwrap();
        let now = fixed_time();
        let input_path = config.paths.state_root.join("brief.txt");
        let artifact_path = config.paths.state_root.join("artifact-1.txt");
        let manifest_artifact_path = config.paths.state_root.join("artifact-2.txt");
        std::fs::write(&input_path, "brief").unwrap();
        std::fs::write(&artifact_path, "artifact-1").unwrap();
        std::fs::write(&manifest_artifact_path, "artifact-2").unwrap();

        db.upsert_session(
            "session-1",
            "Browser session",
            Some("https://chatgpt.com/c/conv-inspect"),
            now + chrono::Duration::minutes(6),
        )
        .unwrap();

        let manifest = ConversationRecoveryManifest {
            conversation_id: "conv-inspect".to_owned(),
            archived_context: vec!["Earlier summarized context".to_owned()],
            recent_turns: vec![
                RecoveryManifestTurn {
                    completed_at: now + chrono::Duration::minutes(1),
                    request_text: "Exact request one".to_owned(),
                    response_excerpt: "Exact response one".to_owned(),
                    input_files: vec!["brief.txt".to_owned()],
                    generated_artifacts: vec!["artifact-1.txt".to_owned()],
                },
                RecoveryManifestTurn {
                    completed_at: now + chrono::Duration::minutes(3),
                    request_text: "Exact request two".to_owned(),
                    response_excerpt: "Recovered response excerpt".to_owned(),
                    input_files: vec!["brief.txt".to_owned()],
                    generated_artifacts: vec!["artifact-2.txt".to_owned()],
                },
                RecoveryManifestTurn {
                    completed_at: now + chrono::Duration::minutes(5),
                    request_text: "CLI request three".to_owned(),
                    response_excerpt: "CLI response three".to_owned(),
                    input_files: vec![],
                    generated_artifacts: vec![],
                },
            ],
            tracked_input_files: vec![RecoveryManifestFile {
                kind: RecoveryManifestFileKind::InputAttachment,
                file_name: "brief.txt".to_owned(),
                local_path: input_path.display().to_string(),
                content_type: Some("text/plain".to_owned()),
                recorded_at: now + chrono::Duration::minutes(3),
            }],
            tracked_generated_artifacts: vec![RecoveryManifestFile {
                kind: RecoveryManifestFileKind::GeneratedArtifact,
                file_name: "artifact-2.txt".to_owned(),
                local_path: manifest_artifact_path.display().to_string(),
                content_type: Some("text/plain".to_owned()),
                recorded_at: now + chrono::Duration::minutes(3),
            }],
            updated_at: now + chrono::Duration::minutes(5),
        };

        db.upsert_conversation(
            "conv-inspect",
            "session-1",
            "Inspection conversation",
            Some("https://chatgpt.com/c/conv-inspect"),
            Some("CLI request three"),
            Some("CLI response three"),
            Some(&manifest),
            now + chrono::Duration::minutes(6),
        )
        .unwrap();
        db.upsert_conversation(
            "conv-summary",
            "session-1",
            "Summary only conversation",
            Some("https://chatgpt.com/c/conv-summary"),
            Some("Last prompt summary"),
            Some("Last response summary"),
            Some(&ConversationRecoveryManifest {
                conversation_id: "conv-summary".to_owned(),
                archived_context: vec!["Only summarized context remains".to_owned()],
                recent_turns: vec![],
                tracked_input_files: vec![],
                tracked_generated_artifacts: vec![],
                updated_at: now + chrono::Duration::minutes(7),
            }),
            now + chrono::Duration::minutes(7),
        )
        .unwrap();

        db.insert_job(
            "job-1",
            JobStatus::Completed,
            Some("ask completed"),
            Some("conv-inspect"),
            now,
            now + chrono::Duration::minutes(1),
        )
        .unwrap();
        db.upsert_job_detail("job-1", JobKind::Ask, Some("Exact request one"), None)
            .unwrap();
        db.update_job_output(
            "job-1",
            Some("Exact response one"),
            Some("https://chatgpt.com/c/conv-inspect"),
            None,
        )
        .unwrap();

        db.insert_artifact(
            &ArtifactMetadata {
                artifact_id: "artifact-1".to_owned(),
                conversation_id: Some("conv-inspect".to_owned()),
                message_id: Some("msg-1".to_owned()),
                file_name: "artifact-1.txt".to_owned(),
                content_type: "text/plain".to_owned(),
                size_bytes: 10,
                sha256: "sha-1".to_owned(),
                storage_path: artifact_path.display().to_string(),
            },
            now + chrono::Duration::minutes(1),
        )
        .unwrap();
        db.link_job_artifact("job-1", "artifact-1").unwrap();

        db.upsert_request_summary(&RequestSummary {
            request_id: "req-1".to_owned(),
            job_id: "job-1".to_owned(),
            model_id: Some("tablinq-instant".to_owned()),
            request_role: Some("primary".to_owned()),
            parent_request_id: None,
            execution_kind: RequestExecutionKind::Ask,
            status: JobStatus::Completed,
            cancelled: false,
            start_time: now,
            end_time: Some(now + chrono::Duration::minutes(1)),
            duration_ms: Some(60_000),
            queue_wait_ms: 0,
            interrupted_at: None,
            interruption_reason: None,
            recovered_at: None,
            recovery_reason: None,
            failed_at: None,
            failure_reason: None,
            recovery_trace: None,
            recovery_packet: None,
            usage: RequestUsage {
                status: "unknown".to_owned(),
                prompt_tokens: None,
                completion_tokens: None,
                total_tokens: None,
            },
            warnings: vec![],
        })
        .unwrap();

        let recovery_trace = RecoveryTrace {
            recovery_available: true,
            recovery_attempted: true,
            recovery_strategy: Some(RecoveryStrategy::ReloadNewConversationReplayWithContext),
            failure_code: Some(ErrorCode::ProviderGenerationFailedNonretryable),
            failure_bundle_id: Some("bundle-1".to_owned()),
            prior_request_id: Some("req-1".to_owned()),
            replacement_request_id: Some("req-2-replay".to_owned()),
            prior_conversation_id: Some("conv-inspect".to_owned()),
            replacement_conversation_id: Some("conv-replay".to_owned()),
            recovery_attempt_count: 1,
        };
        let recovery_packet = RecoveryPacket {
            original_input: "Exact request two".to_owned(),
            mode: Mode::Normal,
            model_class: Some(ModelClass::Thinking),
            effort: Some(Effort::Extended),
            files: vec![input_path.display().to_string()],
            context_summary: Some("Context from recovery packet".to_owned()),
            context_manifest: Some(manifest.clone()),
            recovery_manifest_path: Some(
                config
                    .paths
                    .state_root
                    .join("recovery-manifest.md")
                    .display()
                    .to_string(),
            ),
            replay_files: vec![input_path.display().to_string()],
            prior_request_id: Some("req-1".to_owned()),
            prior_conversation_id: Some("conv-inspect".to_owned()),
            failure_code: ErrorCode::ProviderGenerationFailedNonretryable,
            recovery_attempt_count: 1,
        };

        db.insert_job(
            "job-2",
            JobStatus::Recovered,
            Some("ask recovered"),
            Some("conv-inspect"),
            now + chrono::Duration::minutes(2),
            now + chrono::Duration::minutes(3),
        )
        .unwrap();
        db.upsert_job_detail("job-2", JobKind::Ask, Some("Exact request two"), None)
            .unwrap();
        db.update_job_recovery_metadata("job-2", Some(&recovery_trace), Some(&recovery_packet))
            .unwrap();

        db.upsert_request_summary(&RequestSummary {
            request_id: "req-2".to_owned(),
            job_id: "job-2".to_owned(),
            model_id: Some("tablinq-thinking-extended".to_owned()),
            request_role: Some("primary".to_owned()),
            parent_request_id: None,
            execution_kind: RequestExecutionKind::Ask,
            status: JobStatus::Recovered,
            cancelled: false,
            start_time: now + chrono::Duration::minutes(2),
            end_time: Some(now + chrono::Duration::minutes(3)),
            duration_ms: Some(60_000),
            queue_wait_ms: 100,
            interrupted_at: Some(now + chrono::Duration::minutes(2)),
            interruption_reason: Some("provider generation failed".to_owned()),
            recovered_at: Some(now + chrono::Duration::minutes(3)),
            recovery_reason: Some("fresh conversation replay succeeded".to_owned()),
            failed_at: None,
            failure_reason: None,
            recovery_trace: Some(recovery_trace),
            recovery_packet: Some(recovery_packet),
            usage: RequestUsage {
                status: "unknown".to_owned(),
                prompt_tokens: None,
                completion_tokens: None,
                total_tokens: None,
            },
            warnings: vec!["replayed from preserved context".to_owned()],
        })
        .unwrap();

        db.insert_job(
            "job-3",
            JobStatus::Completed,
            Some("cli ask completed"),
            Some("conv-inspect"),
            now + chrono::Duration::minutes(4),
            now + chrono::Duration::minutes(5),
        )
        .unwrap();
        db.upsert_job_detail("job-3", JobKind::Ask, Some("CLI request three"), None)
            .unwrap();
        db.update_job_output(
            "job-3",
            Some("CLI response three"),
            Some("https://chatgpt.com/c/conv-inspect"),
            None,
        )
        .unwrap();
    }

    #[test]
    fn conversation_detail_assembles_summary_counts_and_latest_ids() {
        let (_dir, config, core) = make_core();
        seed_conversation_fixtures(&config);

        let detail = core
            .find_conversation_detail("conv-inspect")
            .unwrap()
            .expect("conversation detail");

        assert_eq!(detail.summary.conversation_id, "conv-inspect");
        assert_eq!(
            detail.session.as_ref().map(|item| item.session_id.as_str()),
            Some("session-1")
        );
        assert_eq!(detail.latest_request_id.as_deref(), Some("req-2"));
        assert_eq!(detail.latest_job_id.as_deref(), Some("job-3"));
        assert_eq!(detail.counts.request_count, 2);
        assert_eq!(detail.counts.artifact_count, 1);
        assert_eq!(
            detail
                .recovery_manifest
                .as_ref()
                .map(|item| item.archived_context_count),
            Some(1)
        );
        assert_eq!(
            detail.routes.messages,
            "/v1/conversations/conv-inspect/messages"
        );
    }

    #[test]
    fn normalized_history_preserves_order_and_provenance() {
        let (_dir, config, core) = make_core();
        seed_conversation_fixtures(&config);

        let history = core
            .list_conversation_history("conv-inspect", 20, 0)
            .unwrap()
            .expect("history");

        assert_eq!(history.total_items, 7);
        assert_eq!(
            history.messages[0].source_kind,
            ConversationMessageSourceKind::SummaryFallback
        );
        assert_eq!(history.messages[1].role, ConversationMessageRole::User);
        assert_eq!(
            history.messages[1].source_kind,
            ConversationMessageSourceKind::RequestDetail
        );
        assert_eq!(history.messages[2].role, ConversationMessageRole::Assistant);
        assert_eq!(history.messages[2].message_id.as_deref(), Some("msg-1"));
        assert_eq!(history.messages[3].request_id.as_deref(), Some("req-2"));
        assert_eq!(
            history.messages[4].source_kind,
            ConversationMessageSourceKind::RecoveryManifest
        );
        assert_eq!(
            history.messages[4].exactness,
            ConversationMessageExactness::Excerpt
        );
        assert_eq!(history.messages[4].content, "Recovered response excerpt");
        assert_eq!(history.messages[5].request_id, None);
        assert_eq!(history.messages[6].content, "CLI response three");
    }

    #[test]
    fn snapshot_uses_latest_recovery_packet_and_manifest() {
        let (_dir, config, core) = make_core();
        seed_conversation_fixtures(&config);

        let snapshot = core
            .conversation_snapshot("conv-inspect")
            .unwrap()
            .expect("snapshot");

        assert_eq!(snapshot.latest_request_id.as_deref(), Some("req-2"));
        assert_eq!(snapshot.latest_job_id.as_deref(), Some("job-3"));
        assert_eq!(
            snapshot
                .latest_recovery_packet
                .as_ref()
                .map(|packet| packet.original_input.as_str()),
            Some("Exact request two")
        );
        assert_eq!(
            snapshot
                .latest_recovery_trace
                .as_ref()
                .and_then(|trace| trace.replacement_request_id.as_deref()),
            Some("req-2-replay")
        );
        assert_eq!(
            snapshot.context_summary.as_deref(),
            Some("Context from recovery packet")
        );
        assert!(snapshot.suitable_for_fresh_conversation_replay);
        assert_eq!(snapshot.tracked_input_files.len(), 1);
        assert_eq!(
            snapshot
                .recent_messages
                .last()
                .map(|item| item.content.as_str()),
            Some("CLI response three")
        );
    }

    #[test]
    fn summary_only_history_does_not_fabricate_exact_request_turns() {
        let (_dir, config, core) = make_core();
        seed_conversation_fixtures(&config);

        let history = core
            .list_conversation_history("conv-summary", 20, 0)
            .unwrap()
            .expect("history");

        assert_eq!(history.total_items, 1);
        assert_eq!(
            history.messages[0].source_kind,
            ConversationMessageSourceKind::SummaryFallback
        );
        assert_eq!(
            history.messages[0].exactness,
            ConversationMessageExactness::Summary
        );
        assert_eq!(
            history.messages[0].content,
            "Only summarized context remains"
        );
    }

    #[test]
    fn session_conversations_return_session_scoped_results() {
        let (_dir, config, core) = make_core();
        seed_conversation_fixtures(&config);

        let sessions = core
            .list_session_conversations("session-1")
            .unwrap()
            .expect("session conversations");

        assert_eq!(sessions.session.session_id, "session-1");
        assert_eq!(sessions.conversations.len(), 2);
        assert_eq!(sessions.conversations[0].conversation_id, "conv-summary");
        assert_eq!(sessions.conversations[1].conversation_id, "conv-inspect");
    }
}
