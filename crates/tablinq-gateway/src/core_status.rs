use anyhow::Result;
use gptd::{AppConfig, CoreService};
use shared_protocol::{
    AuthSource, BrowserConnectionMode, BrowserOwner, BrowserStatusSnapshot, DaemonRequest,
    DaemonResponse, DoctorReport, HealthCheck,
};

#[derive(Debug, Clone)]
pub(crate) struct EffectiveGatewayAdminSnapshot {
    pub(crate) health: HealthCheck,
    pub(crate) browser: BrowserStatusSnapshot,
    pub(crate) browser_warning: Option<String>,
    pub(crate) daemon_health: Option<HealthCheck>,
}

pub(crate) fn gateway_live_health_snapshot(core: &CoreService) -> Result<HealthCheck> {
    core.health_snapshot()
}

pub(crate) fn daemon_diagnostic_health_snapshot(config: &AppConfig) -> Option<HealthCheck> {
    if !gptd::daemon_is_responsive(&config.paths.ipc_endpoint) {
        return None;
    }
    match gptd::send_request(&config.paths.ipc_endpoint, &DaemonRequest::Ping).ok()? {
        DaemonResponse::Health(health) => Some(health),
        _ => None,
    }
}

pub(crate) fn effective_gateway_admin_snapshot(
    config: &AppConfig,
    core: &CoreService,
) -> Result<EffectiveGatewayAdminSnapshot> {
    let daemon_health = daemon_diagnostic_health_snapshot(config);
    if let Some(report) = daemon_doctor_report(config) {
        return Ok(admin_snapshot_from_doctor_report(report, daemon_health));
    }

    core_admin_snapshot(config, core, daemon_health)
}

pub(crate) fn effective_doctor_report(
    config: &AppConfig,
    core: &CoreService,
) -> Result<DoctorReport> {
    if let Some(report) = daemon_doctor_report(config) {
        Ok(report)
    } else {
        core.doctor_report()
    }
}

pub(crate) fn degraded_browser_status_snapshot(config: &AppConfig) -> BrowserStatusSnapshot {
    let attach_mode = config.browser_connection_mode();
    BrowserStatusSnapshot {
        running: false,
        browser_kind: degraded_browser_kind(config),
        profile_dir: config.paths.browser_profile_dir.display().to_string(),
        attach_profile_dir: None,
        attach_profile_note: None,
        downloads_dir: config.paths.downloads_dir.display().to_string(),
        target_url: config.browser_target_url().to_owned(),
        current_url: None,
        page_title: None,
        page_ready: false,
        authenticated: false,
        reload_count: 0,
        screenshot_path: None,
        browser_path: config.browser_path().map(ToOwned::to_owned),
        extension_connected: false,
        capability_report: None,
        attach_mode,
        browser_owner: match attach_mode {
            BrowserConnectionMode::ManagedOnly => BrowserOwner::Managed,
            BrowserConnectionMode::AttachFirst | BrowserConnectionMode::ManagedFallback => {
                BrowserOwner::External
            }
        },
        cdp_attached: false,
        target_tab_found: false,
        target_tab_id: None,
        target_workspace_key: None,
        debug_endpoint: Some(config.browser_debug_endpoint()),
        auth_source: AuthSource::Unknown,
        extension_required: config.browser_extension_required(),
        last_capability_report_at: None,
        action: None,
        browser_process_closed: None,
        reason: None,
    }
}

fn daemon_doctor_report(config: &AppConfig) -> Option<DoctorReport> {
    if !gptd::daemon_is_responsive(&config.paths.ipc_endpoint) {
        return None;
    }
    match gptd::send_request(&config.paths.ipc_endpoint, &DaemonRequest::Doctor).ok()? {
        DaemonResponse::Doctor(report) => Some(report),
        _ => None,
    }
}

fn admin_snapshot_from_doctor_report(
    report: DoctorReport,
    daemon_health: Option<HealthCheck>,
) -> EffectiveGatewayAdminSnapshot {
    admin_snapshot_from_health_and_browser(report.health, report.browser, daemon_health)
}

fn admin_snapshot_from_health_and_browser(
    health: HealthCheck,
    browser: BrowserStatusSnapshot,
    daemon_health: Option<HealthCheck>,
) -> EffectiveGatewayAdminSnapshot {
    let diagnostic_daemon_health = diagnostic_daemon_health(daemon_health, &health);
    EffectiveGatewayAdminSnapshot {
        daemon_health: diagnostic_daemon_health,
        health,
        browser,
        browser_warning: None,
    }
}

fn core_admin_snapshot(
    config: &AppConfig,
    core: &CoreService,
    daemon_health: Option<HealthCheck>,
) -> Result<EffectiveGatewayAdminSnapshot> {
    match core.doctor_report() {
        Ok(report) => Ok(admin_snapshot_from_doctor_report(report, daemon_health)),
        Err(err) => {
            let health = gateway_live_health_snapshot(core)?;
            let browser = match core.browser_profile_info() {
                Ok(browser) => browser,
                Err(profile_err) => {
                    return Ok(EffectiveGatewayAdminSnapshot {
                        daemon_health: diagnostic_daemon_health(daemon_health, &health),
                        health,
                        browser: degraded_browser_status_snapshot(config),
                        browser_warning: Some(format!(
                            "{err:#}; browser_profile_info: {profile_err:#}"
                        )),
                    });
                }
            };
            Ok(EffectiveGatewayAdminSnapshot {
                daemon_health: diagnostic_daemon_health(daemon_health, &health),
                health,
                browser,
                browser_warning: Some(format!("{err:#}")),
            })
        }
    }
}

fn diagnostic_daemon_health(
    daemon_health: Option<HealthCheck>,
    health: &HealthCheck,
) -> Option<HealthCheck> {
    daemon_health.filter(|daemon| daemon_health_conflicts(health, daemon))
}

fn daemon_health_conflicts(live: &HealthCheck, daemon: &HealthCheck) -> bool {
    live.status != daemon.status
        || live.browser_connected != daemon.browser_connected
        || live.extension_connected != daemon.extension_connected
        || live.authenticated != daemon.authenticated
}

fn degraded_browser_kind(config: &AppConfig) -> String {
    if let Some(path) = config.browser_path() {
        let lower = path.to_ascii_lowercase();
        if lower.contains("msedge") || lower.contains("microsoft edge") {
            return "msedge".to_owned();
        }
        if lower.contains("chrome") {
            return "chrome".to_owned();
        }
    }
    match config.browser_channel().to_ascii_lowercase().as_str() {
        "chrome" => "chrome".to_owned(),
        "msedge" | "edge" => "msedge".to_owned(),
        value if !value.is_empty() => value.to_owned(),
        _ => {
            if cfg!(windows) {
                "msedge".to_owned()
            } else {
                "chromium".to_owned()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use shared_protocol::HealthStatus;

    #[test]
    fn daemon_health_is_exposed_only_as_diagnostic_when_it_conflicts_with_gateway_core() {
        let live = HealthCheck {
            status: HealthStatus::Ok,
            checked_at: Utc::now(),
            browser_connected: true,
            extension_connected: true,
            authenticated: true,
            details: Some("gateway core attached".to_owned()),
        };

        let identical = HealthCheck {
            status: HealthStatus::Ok,
            checked_at: Utc::now(),
            browser_connected: true,
            extension_connected: true,
            authenticated: true,
            details: Some("daemon agrees".to_owned()),
        };

        assert!(diagnostic_daemon_health(Some(identical), &live).is_none());
    }

    #[test]
    fn admin_snapshot_uses_daemon_doctor_report_without_busy_warning() {
        let health = attached_health();
        let browser = attached_browser_snapshot();
        let snapshot = admin_snapshot_from_health_and_browser(
            health.clone(),
            browser.clone(),
            Some(health.clone()),
        );

        assert_eq!(snapshot.health, health);
        assert_eq!(snapshot.browser, browser);
        assert!(snapshot.browser_warning.is_none());
        assert!(snapshot.daemon_health.is_none());
    }

    #[test]
    fn admin_snapshot_prefers_authoritative_attached_state_over_stale_degraded_state() {
        let authoritative_health = attached_health();
        let authoritative_browser = attached_browser_snapshot();
        let stale_degraded_health = HealthCheck {
            status: HealthStatus::Degraded,
            checked_at: Utc::now(),
            browser_connected: false,
            extension_connected: false,
            authenticated: false,
            details: Some("stale gateway-local browser not connected".to_owned()),
        };

        let snapshot = admin_snapshot_from_health_and_browser(
            authoritative_health,
            authoritative_browser,
            Some(stale_degraded_health),
        );

        assert_eq!(snapshot.health.status, HealthStatus::Ok);
        assert_eq!(snapshot.health.browser_connected, true);
        assert_eq!(snapshot.health.extension_connected, true);
        assert_eq!(snapshot.health.authenticated, true);
        assert_eq!(snapshot.browser.running, true);
        assert_eq!(snapshot.browser.cdp_attached, true);
        assert_eq!(snapshot.browser.extension_connected, true);
        assert_eq!(snapshot.browser.page_ready, true);
        assert_eq!(snapshot.browser.target_tab_found, true);
        assert_eq!(snapshot.browser.auth_source, AuthSource::Page);
    }

    fn attached_health() -> HealthCheck {
        HealthCheck {
            status: HealthStatus::Ok,
            checked_at: Utc::now(),
            browser_connected: true,
            extension_connected: true,
            authenticated: true,
            details: Some("daemon attached".to_owned()),
        }
    }

    fn attached_browser_snapshot() -> BrowserStatusSnapshot {
        BrowserStatusSnapshot {
            running: true,
            browser_kind: "chrome".to_owned(),
            profile_dir: "profile".to_owned(),
            attach_profile_dir: None,
            attach_profile_note: None,
            downloads_dir: "downloads".to_owned(),
            target_url: "https://chatgpt.com".to_owned(),
            current_url: Some("https://chatgpt.com/c/test".to_owned()),
            page_title: Some("ChatGPT".to_owned()),
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
            target_tab_id: Some("tab-1".to_owned()),
            target_workspace_key: None,
            debug_endpoint: Some("http://127.0.0.1:9222".to_owned()),
            auth_source: AuthSource::Page,
            extension_required: true,
            last_capability_report_at: None,
            action: None,
            browser_process_closed: None,
            reason: None,
        }
    }
}
