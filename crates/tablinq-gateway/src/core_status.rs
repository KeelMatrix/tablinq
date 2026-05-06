use anyhow::Result;
use gptd::{AppConfig, CoreService};
use shared_protocol::{
    AuthSource, BrowserConnectionMode, BrowserOwner, BrowserStatusSnapshot, DaemonRequest,
    DaemonResponse, DoctorReport, HealthCheck,
};

#[derive(Debug, Clone)]
pub(crate) struct EffectiveGatewayHealthSnapshot {
    pub(crate) health: HealthCheck,
    pub(crate) daemon_health: Option<HealthCheck>,
}

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

pub(crate) fn effective_gateway_health_snapshot(
    config: &AppConfig,
    core: &CoreService,
) -> Result<EffectiveGatewayHealthSnapshot> {
    let health = gateway_live_health_snapshot(core)?;
    Ok(merge_gateway_health_snapshot(
        health,
        daemon_diagnostic_health_snapshot(config),
    ))
}

pub(crate) fn effective_gateway_admin_snapshot(
    config: &AppConfig,
    core: &CoreService,
) -> Result<EffectiveGatewayAdminSnapshot> {
    let daemon_health = daemon_diagnostic_health_snapshot(config);
    match core.doctor_report() {
        Ok(report) => {
            let health = report.health.clone();
            Ok(EffectiveGatewayAdminSnapshot {
                daemon_health: diagnostic_daemon_health(daemon_health, &health),
                health,
                browser: report.browser,
                browser_warning: None,
            })
        }
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

fn merge_gateway_health_snapshot(
    health: HealthCheck,
    daemon_health: Option<HealthCheck>,
) -> EffectiveGatewayHealthSnapshot {
    let daemon_health = diagnostic_daemon_health(daemon_health, &health);
    EffectiveGatewayHealthSnapshot {
        health,
        daemon_health,
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
    fn effective_health_snapshot_prefers_gateway_core_browser_state_over_stale_daemon_health() {
        let live = HealthCheck {
            status: HealthStatus::Ok,
            checked_at: Utc::now(),
            browser_connected: true,
            extension_connected: true,
            authenticated: true,
            details: Some("gateway core attached".to_owned()),
        };
        let stale = HealthCheck {
            status: HealthStatus::Degraded,
            checked_at: Utc::now(),
            browser_connected: false,
            extension_connected: false,
            authenticated: false,
            details: Some("daemon stale".to_owned()),
        };

        let snapshot = merge_gateway_health_snapshot(live.clone(), Some(stale.clone()));

        assert_eq!(snapshot.health, live);
        assert_eq!(snapshot.daemon_health, Some(stale));
    }

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

        let snapshot = merge_gateway_health_snapshot(live, Some(identical));

        assert!(snapshot.daemon_health.is_none());
    }
}
