use crate::config::AppConfig;
use crate::db::Database;
use crate::ipc;
use crate::lockfile;
use crate::logging::JsonLogger;
use anyhow::{Context, Result};
use chrono::Utc;
use interprocess::local_socket::traits::Listener as LocalSocketListenerTrait;
use serde_json::json;
use shared_protocol::{
    AckEvent, DaemonErrorResponse, DaemonRequest, DaemonResponse, DaemonStatusSnapshot,
    DoctorReport, ErrorCode, HealthCheck, HealthStatus,
};

pub fn run_service(config: AppConfig) -> Result<()> {
    let logger = JsonLogger::new(&config.paths.log_path)?;
    logger.info(
        "daemon starting",
        json!({
            "state_root": config.paths.state_root,
            "ipc_endpoint": config.paths.ipc_endpoint,
        }),
    )?;

    let _lock = lockfile::acquire(&config.paths)?;
    let db = Database::open(&config.paths.db_path)?;
    let listener = ipc::bind(&config.paths.ipc_endpoint)?;
    let started_at = Utc::now();

    logger.info(
        "daemon ready",
        json!({
            "db_path": config.paths.db_path,
            "warm_mode": config.warm_mode(),
        }),
    )?;

    loop {
        let stream = listener.accept().context("accept daemon connection")?;
        let (request, stream) = ipc::read_request(stream)?;
        logger.info("daemon request", json!({ "request": request }))?;

        let should_shutdown = matches!(request, DaemonRequest::Shutdown);
        let response = handle_request(&config, &db, started_at, request)?;
        ipc::write_response(stream, &response)?;

        if should_shutdown {
            logger.warn("daemon stopping", json!({ "reason": "shutdown request" }))?;
            break;
        }
    }

    Ok(())
}

fn handle_request(
    config: &AppConfig,
    db: &Database,
    started_at: chrono::DateTime<Utc>,
    request: DaemonRequest,
) -> Result<DaemonResponse> {
    Ok(match request {
        DaemonRequest::Ping => DaemonResponse::Health(health_snapshot(config)),
        DaemonRequest::Status => DaemonResponse::Status(status_snapshot(config, started_at)),
        DaemonRequest::Shutdown => DaemonResponse::Ack(AckEvent {
            accepted: true,
            message: "daemon stopping".to_owned(),
        }),
        DaemonRequest::Doctor => DaemonResponse::Doctor(DoctorReport {
            health: health_snapshot(config),
            status: status_snapshot(config, started_at),
            config: config.snapshot(),
        }),
        DaemonRequest::ConfigShow => DaemonResponse::Config(config.snapshot()),
        DaemonRequest::SessionsList => DaemonResponse::Sessions(db.list_sessions()?),
        DaemonRequest::JobsList => DaemonResponse::Jobs(db.list_jobs()?),
    })
}

pub fn status_snapshot(
    config: &AppConfig,
    started_at: chrono::DateTime<Utc>,
) -> DaemonStatusSnapshot {
    DaemonStatusSnapshot {
        pid: std::process::id(),
        started_at,
        state_root: config.paths.state_root.display().to_string(),
        ipc_endpoint: config.paths.ipc_endpoint.clone(),
        lock_path: config.paths.lock_path.display().to_string(),
        db_path: config.paths.db_path.display().to_string(),
        warm_mode: config.warm_mode(),
    }
}

pub fn health_snapshot(config: &AppConfig) -> HealthCheck {
    HealthCheck {
        status: HealthStatus::Ok,
        checked_at: Utc::now(),
        browser_connected: false,
        extension_connected: false,
        authenticated: false,
        details: Some(if config.warm_mode() {
            "control plane ready; browser warm mode enabled in config".to_owned()
        } else {
            "control plane ready; browser runtime not started yet".to_owned()
        }),
    }
}

pub fn map_error(err: anyhow::Error) -> DaemonResponse {
    DaemonResponse::Error(DaemonErrorResponse {
        error_code: ErrorCode::Transport,
        message: format!("{err:#}"),
    })
}
