use super::support::{default_selector_pack_summary, lock_browser};
use super::{ARTIFACT_RETENTION_LIMIT, CoreService, error_code_for_message};
use crate::Database;
use crate::browser::BrowserDiagnosticsSnapshot;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;
use sha2::{Digest, Sha256};
use shared_protocol::{
    ArtifactList, ArtifactMetadata, ArtifactPullResult, ArtifactSummary, FailureBundle,
    GatewayRequestContext, PullArtifactCommand,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

const EVIDENCE_ARTIFACT_CONTENT_TYPE: &str = "application/tablinq-browser-evidence+json";
const DIAGNOSTICS_CAPTURE_TIMEOUT: Duration = Duration::from_secs(3);

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

    pub fn capture_browser_evidence(&self, request_id: &str) -> Result<ArtifactMetadata> {
        let db = self.open_db()?;
        let created_at = Utc::now();
        let evidence_dir = self.config.paths.crash_bundles_dir.join(format!(
            "evidence-{request_id}-{}",
            created_at.timestamp_millis()
        ));
        fs::create_dir_all(&evidence_dir)
            .with_context(|| format!("create {}", evidence_dir.display()))?;

        let diagnostics = capture_diagnostics_snapshot(&self.browser, &evidence_dir);
        persist_browser_evidence(
            &db,
            self,
            request_id,
            created_at,
            evidence_dir.as_path(),
            diagnostics,
        )
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
        persist_failure_bundle(
            &db,
            self,
            mode,
            message,
            gateway_request,
            created_at,
            bundle_id,
            bundle_dir.as_path(),
            diagnostics,
        )
    }

    pub fn capture_gateway_failure_artifacts(
        &self,
        request_id: &str,
        mode: Option<shared_protocol::Mode>,
        message: &str,
        gateway_request: Option<GatewayRequestContext>,
        include_request_evidence: bool,
    ) -> Result<Option<String>> {
        let db = self.open_db()?;
        let created_at = Utc::now();
        let bundle_id = format!("bundle-{}", created_at.timestamp_millis());
        let capture_dir = self.config.paths.crash_bundles_dir.join(&bundle_id);
        fs::create_dir_all(&capture_dir)
            .with_context(|| format!("create {}", capture_dir.display()))?;

        let diagnostics = capture_diagnostics_snapshot(&self.browser, &capture_dir);
        if include_request_evidence {
            persist_browser_evidence(
                &db,
                self,
                request_id,
                created_at,
                capture_dir.as_path(),
                clone_diagnostics(&diagnostics),
            )?;
        }
        if let Some(gateway_request) = gateway_request {
            let bundle = persist_failure_bundle(
                &db,
                self,
                mode,
                message,
                Some(gateway_request),
                created_at,
                bundle_id,
                capture_dir.as_path(),
                diagnostics,
            )?;
            Ok(Some(bundle.bundle_id))
        } else {
            Ok(None)
        }
    }
}

fn persist_browser_evidence(
    db: &Database,
    core: &CoreService,
    request_id: &str,
    created_at: chrono::DateTime<Utc>,
    output_dir: &Path,
    diagnostics: BrowserDiagnosticsSnapshot,
) -> Result<ArtifactMetadata> {
    let request = core.request_summary(request_id)?;
    let job = core.job_detail(&request.job_id)?;
    let payload = BrowserEvidencePayload {
        request_id: request.request_id.clone(),
        job_id: request.job_id.clone(),
        captured_at: created_at,
        current_url: diagnostics.current_url.clone(),
        screenshot_path: diagnostics.screenshot_path.clone(),
        dom_snapshot: diagnostics.dom_snapshot,
    };
    let artifact_path = output_dir.join("evidence.json");
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
        sha256: sha256_hex(&payload_bytes),
        storage_path: artifact_path.display().to_string(),
    };
    db.insert_artifact(&artifact, created_at)?;
    db.link_job_artifact(&request.job_id, &artifact.artifact_id)?;
    db.prune_artifacts(ARTIFACT_RETENTION_LIMIT)?;
    Ok(artifact)
}

#[allow(clippy::too_many_arguments)]
fn persist_failure_bundle(
    db: &Database,
    core: &CoreService,
    mode: Option<shared_protocol::Mode>,
    message: &str,
    gateway_request: Option<GatewayRequestContext>,
    created_at: chrono::DateTime<Utc>,
    bundle_id: String,
    output_dir: &Path,
    diagnostics: BrowserDiagnosticsSnapshot,
) -> Result<FailureBundle> {
    let dom_snapshot_path = output_dir.join("dom.json");
    fs::write(&dom_snapshot_path, diagnostics.dom_snapshot.as_bytes())
        .with_context(|| format!("write {}", dom_snapshot_path.display()))?;
    let aria_snapshot_path = output_dir.join("aria.json");
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
        logs_path: Some(core.config.paths.log_path.display().to_string()),
        strategy_attempts: diagnostics.strategy_attempts,
        capability_matrix: diagnostics.capability_matrix,
        gateway_request,
    };
    let bundle_json_path = output_dir.join("bundle.json");
    fs::write(&bundle_json_path, serde_json::to_vec_pretty(&bundle)?)
        .with_context(|| format!("write {}", bundle_json_path.display()))?;
    db.insert_failure_bundle(&bundle)?;
    Ok(bundle)
}

fn clone_diagnostics(diagnostics: &BrowserDiagnosticsSnapshot) -> BrowserDiagnosticsSnapshot {
    BrowserDiagnosticsSnapshot {
        current_url: diagnostics.current_url.clone(),
        capability_matrix: diagnostics.capability_matrix.clone(),
        strategy_attempts: diagnostics.strategy_attempts.clone(),
        dom_snapshot: diagnostics.dom_snapshot.clone(),
        aria_snapshot: diagnostics.aria_snapshot.clone(),
        screenshot_path: diagnostics.screenshot_path.clone(),
        selector_pack: diagnostics.selector_pack.clone(),
        strategy_telemetry: diagnostics.strategy_telemetry.clone(),
        browser_detected: diagnostics.browser_detected,
        detected_browser_path: diagnostics.detected_browser_path.clone(),
        profile_accessible: diagnostics.profile_accessible,
        profile_access_error: diagnostics.profile_access_error.clone(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut text, "{byte:02x}");
    }
    text
}

fn capture_diagnostics_snapshot(
    browser: &std::sync::Arc<std::sync::Mutex<crate::browser::BrowserManager>>,
    output_dir: &Path,
) -> BrowserDiagnosticsSnapshot {
    let browser = std::sync::Arc::clone(browser);
    let output_dir = output_dir.to_path_buf();
    let worker_output_dir = output_dir.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let snapshot = lock_browser(&browser)
            .and_then(|mut manager| manager.diagnostics(Some(&worker_output_dir)))
            .unwrap_or_else(|_| placeholder_diagnostics_snapshot(&worker_output_dir));
        let _ = sender.send(snapshot);
    });

    receiver
        .recv_timeout(DIAGNOSTICS_CAPTURE_TIMEOUT)
        .unwrap_or_else(|_| placeholder_diagnostics_snapshot(output_dir.as_path()))
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
