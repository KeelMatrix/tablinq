use crate::core_status::{effective_doctor_report, effective_gateway_admin_snapshot};
use crate::http::HttpResponse;
use crate::json::json_response;
use crate::process_control::GatewayStatusSnapshot;
use crate::scheduler::SchedulerSnapshot;
use anyhow::Result;
use gptd::{AppConfig, CoreService};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};

const CLEAR_SAFETY_MESSAGE: &str =
    "Refusing to clear Tablinq state. Re-run with all=true and force=true.";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClearStateRequest {
    #[serde(default)]
    all: bool,
    #[serde(default)]
    force: bool,
}

pub(crate) fn handle_health_request(
    config: &AppConfig,
    core: &CoreService,
    snapshot: &GatewayStatusSnapshot,
    scheduler_status: &SchedulerSnapshot,
    request_id: &str,
) -> Result<HttpResponse> {
    let admin = effective_gateway_admin_snapshot(config, core)?;
    Ok(json_response(
        200,
        json!({
            "service": "tablinq-gateway",
            "request_id": request_id,
            "status": admin.health.status,
            "health": admin.health,
            "daemon_health": admin.daemon_health,
            "gateway": snapshot,
            "scheduler": scheduler_status,
        }),
    ))
}

pub(crate) fn handle_status_request(
    core: &CoreService,
    snapshot: &GatewayStatusSnapshot,
    scheduler_status: &SchedulerSnapshot,
    request_id: &str,
) -> HttpResponse {
    json_response(
        200,
        json!({
            "object": "tablinq.status",
            "request_id": request_id,
            "data": {
                "gateway": snapshot,
                "daemon": core.status_snapshot(),
                "scheduler": scheduler_status,
            },
        }),
    )
}

pub(crate) fn handle_doctor_request(
    config: &AppConfig,
    core: &CoreService,
    scheduler_status: &SchedulerSnapshot,
    request_id: &str,
) -> Result<HttpResponse> {
    let doctor = effective_doctor_report(config, core)?;
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.doctor",
            "request_id": request_id,
            "data": doctor,
            "scheduler": scheduler_status,
        }),
    ))
}

pub(crate) fn handle_config_request(core: &CoreService, request_id: &str) -> HttpResponse {
    json_response(
        200,
        json!({
            "object": "tablinq.config",
            "request_id": request_id,
            "data": core.config_snapshot(),
        }),
    )
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
    let admin = effective_gateway_admin_snapshot(config, core)?;
    Ok(json_response(
        200,
        json!({
            "request_id": request_id,
            "gateway": snapshot,
            "scheduler": scheduler_status,
            "health": admin.health,
            "browser": admin.browser,
            "browser_warning": admin.browser_warning.map_or(Value::Null, Value::String),
            "daemon_health": admin.daemon_health,
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

pub(crate) fn handle_admin_clear_request(
    core: &CoreService,
    body: &[u8],
    request_id: &str,
) -> Result<HttpResponse> {
    let request = if body.is_empty() {
        ClearStateRequest {
            all: false,
            force: false,
        }
    } else {
        match serde_json::from_slice::<ClearStateRequest>(body) {
            Ok(request) => request,
            Err(err) => {
                return Ok(json_response(
                    400,
                    json!({
                        "request_id": request_id,
                        "error": "invalid_request_json",
                        "message": format!("{err:#}"),
                    }),
                ));
            }
        }
    };

    if !(request.all && request.force) {
        return Ok(json_response(
            400,
            json!({
                "request_id": request_id,
                "error": "clear_requires_all_force",
                "message": CLEAR_SAFETY_MESSAGE,
            }),
        ));
    }

    let result = core.clear_all_state()?;
    Ok(json_response(
        200,
        json!({
            "request_id": request_id,
            "cleared": true,
            "result": result,
        }),
    ))
}
