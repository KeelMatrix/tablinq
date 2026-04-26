use super::support::{default_selector_pack_summary, lock_browser};
use super::{ARTIFACT_RETENTION_LIMIT, CoreService, error_code_for_message};
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
