use crate::core_status::{
    degraded_browser_status_snapshot, effective_doctor_report, effective_health_snapshot,
};
use crate::http::HttpResponse;
use crate::json::json_response;
use crate::process_control::GatewayStatusSnapshot;
use crate::scheduler::SchedulerSnapshot;
use anyhow::Result;
use gptd::{AppConfig, CoreService};
use serde_json::{Value, json};
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
