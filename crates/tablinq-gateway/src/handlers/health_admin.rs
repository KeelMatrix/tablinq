use crate::http::HttpResponse;
use crate::json::json_response;
use crate::process_control::GatewayStatusSnapshot;
use crate::scheduler::SchedulerSnapshot;
use anyhow::Result;
use gptd::{AppConfig, CoreService};
use serde_json::{Value, json};
use shared_protocol::{
    AuthSource, BrowserConnectionMode, BrowserOwner, BrowserStatusSnapshot, DaemonRequest,
    DaemonResponse, DoctorReport, HealthCheck,
};
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) fn handle_health_request(
    config: &AppConfig,
    core: &CoreService,
    snapshot: &GatewayStatusSnapshot,
    scheduler_status: &SchedulerSnapshot,
    request_id: &str,
) -> Result<HttpResponse> {
    let health = effective_health_snapshot(config, core)?;
    Ok(json_response(
        200,
        json!({
            "service": "tablinq-gateway",
            "request_id": request_id,
            "status": health.status,
            "health": health,
            "gateway": snapshot,
            "scheduler": scheduler_status,
        }),
    ))
}

pub(crate) fn handle_ready_request(
    snapshot: &GatewayStatusSnapshot,
    scheduler_status: &SchedulerSnapshot,
    request_id: &str,
) -> HttpResponse {
    json_response(
        200,
        json!({
            "service": "tablinq-gateway",
            "request_id": request_id,
            "ready": true,
            "gateway": snapshot,
            "scheduler": scheduler_status,
        }),
    )
}

pub(crate) fn handle_admin_status_request(
    config: &AppConfig,
    core: &CoreService,
    snapshot: &GatewayStatusSnapshot,
    scheduler_status: &SchedulerSnapshot,
    request_id: &str,
) -> Result<HttpResponse> {
    let health = effective_health_snapshot(config, core)?;
    let (browser, browser_warning) = match effective_doctor_report(config, core) {
        Ok(doctor) => (doctor.browser, Value::Null),
        Err(err) => (
            degraded_browser_status_snapshot(config),
            Value::String(format!("{err:#}")),
        ),
    };
    Ok(json_response(
        200,
        json!({
            "request_id": request_id,
            "gateway": snapshot,
            "scheduler": scheduler_status,
            "health": health,
            "browser": browser,
            "browser_warning": browser_warning,
        }),
    ))
}

pub(crate) fn handle_admin_shutdown_request(
    shutdown: &AtomicBool,
    request_id: &str,
) -> HttpResponse {
    shutdown.store(true, Ordering::SeqCst);
    json_response(
        200,
        json!({
            "request_id": request_id,
            "accepted": true,
        }),
    )
}

pub(crate) fn daemon_health_snapshot(config: &AppConfig) -> Option<HealthCheck> {
    if !gptd::daemon_is_responsive(&config.paths.ipc_endpoint) {
        return None;
    }
    match gptd::send_request(&config.paths.ipc_endpoint, &DaemonRequest::Ping).ok()? {
        DaemonResponse::Health(health) => Some(health),
        _ => None,
    }
}

pub(crate) fn daemon_doctor_report(config: &AppConfig) -> Option<DoctorReport> {
    if !gptd::daemon_is_responsive(&config.paths.ipc_endpoint) {
        return None;
    }
    match gptd::send_request(&config.paths.ipc_endpoint, &DaemonRequest::Doctor).ok()? {
        DaemonResponse::Doctor(report) => Some(report),
        _ => None,
    }
}

pub(crate) fn effective_health_snapshot(
    config: &AppConfig,
    core: &CoreService,
) -> Result<HealthCheck> {
    if let Some(health) = daemon_health_snapshot(config) {
        Ok(health)
    } else {
        core.health_snapshot()
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
