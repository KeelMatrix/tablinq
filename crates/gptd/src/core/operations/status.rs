#![allow(unused_imports)]
use super::super::CoreService;
use super::super::support::{
    browser_busy_message, browser_session_identity, default_selector_pack_summary, health_snapshot,
    is_attach_primary_mode, recovery_guidance, resolve_conversation_target_url, try_lock_browser,
};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use shared_protocol::{
    BrowserDoctor, BrowserOpenRequest, BrowserStatusSnapshot, CapabilitySnapshot, ConfigSnapshot,
    ConversationList, ConversationSelectCommand, ConversationSelection, ConversationSummary,
    DoctorReport, SessionConversationList, SessionList,
};

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
            return Ok(super::super::support::busy_health_snapshot());
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
}
