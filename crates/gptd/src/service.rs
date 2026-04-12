use crate::browser::BrowserManager;
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
    AckEvent, AskCommand, BrowserOpenRequest, CapabilitySnapshot, DaemonErrorResponse, DaemonFrame,
    DaemonRequest, DaemonResponse, DaemonStatusSnapshot, DoctorReport, ErrorCode, EventPayload,
    HealthCheck, HealthStatus, JobState, JobStatus,
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
    let mut browser = BrowserManager::new(&config);

    if config.warm_mode() {
        if let Err(err) = browser.open(BrowserOpenRequest {
            target_url: None,
            browser_path: None,
            headless: None,
        }) {
            logger.warn(
                "warm browser open failed",
                json!({
                    "error": format!("{err:#}"),
                }),
            )?;
        }
    }

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
        if let DaemonRequest::Ask(command) = request {
            if let Err(err) =
                handle_ask_request(&config, &db, &mut browser, &stream, started_at, command)
            {
                logger.warn(
                    "daemon request failed",
                    json!({
                        "error": format!("{err:#}"),
                    }),
                )?;
                ipc::write_frame(&stream, &DaemonFrame::Response(map_error(err)))?;
            }
            continue;
        }
        let response = match handle_request(&config, &db, &mut browser, started_at, request) {
            Ok(response) => response,
            Err(err) => {
                logger.warn(
                    "daemon request failed",
                    json!({
                        "error": format!("{err:#}"),
                    }),
                )?;
                map_error(err)
            }
        };
        ipc::write_response(stream, &response)?;

        if should_shutdown {
            logger.warn("daemon stopping", json!({ "reason": "shutdown request" }))?;
            break;
        }
    }

    Ok(())
}

fn handle_ask_request(
    _config: &AppConfig,
    db: &Database,
    browser: &mut BrowserManager,
    stream: &interprocess::local_socket::Stream,
    _started_at: chrono::DateTime<Utc>,
    command: AskCommand,
) -> Result<()> {
    let now = Utc::now();
    let job_id = format!("job-{}", now.timestamp_millis());
    let conversation_id = match command.conversation_id.clone() {
        Some(value) => Some(value),
        None => db.latest_conversation_id()?,
    };
    let mut effective = command.clone();
    effective.conversation_id = conversation_id.clone();

    db.insert_job(
        &job_id,
        JobStatus::Starting,
        Some("starting ask"),
        effective.conversation_id.as_deref(),
        now,
        now,
    )?;
    emit_job_state(
        stream,
        JobState {
            job_id: job_id.clone(),
            status: JobStatus::Starting,
            message: Some("starting ask".to_owned()),
            conversation_id: effective.conversation_id.clone(),
            updated_at: now,
            artifacts: vec![],
        },
    )?;

    db.insert_job(
        &job_id,
        JobStatus::Running,
        Some("streaming response"),
        effective.conversation_id.as_deref(),
        now,
        now,
    )?;
    emit_job_state(
        stream,
        JobState {
            job_id: job_id.clone(),
            status: JobStatus::Running,
            message: Some("streaming response".to_owned()),
            conversation_id: effective.conversation_id.clone(),
            updated_at: now,
            artifacts: vec![],
        },
    )?;

    match browser.ask(&job_id, &effective, |event| {
        ipc::write_frame(stream, &DaemonFrame::Event(event.clone()))?;
        Ok(())
    }) {
        Ok(result) => {
            let completed_at = result.completed_at;
            db.insert_job(
                &job_id,
                JobStatus::Completed,
                Some("ask completed"),
                Some(&result.conversation_id),
                now,
                completed_at,
            )?;
            db.upsert_session(
                "browser-primary",
                "Managed browser",
                Some(&result.current_url),
                completed_at,
            )?;
            db.upsert_conversation(
                &result.conversation_id,
                "browser-primary",
                "Managed conversation",
                Some(&result.current_url),
                Some(&effective.prompt),
                Some(&result.output_text),
                completed_at,
            )?;
            emit_job_state(
                stream,
                JobState {
                    job_id: job_id.clone(),
                    status: JobStatus::Completed,
                    message: Some("ask completed".to_owned()),
                    conversation_id: Some(result.conversation_id.clone()),
                    updated_at: completed_at,
                    artifacts: vec![],
                },
            )?;
            ipc::write_frame(stream, &DaemonFrame::Response(DaemonResponse::Ask(result)))?;
            Ok(())
        }
        Err(err) => {
            let failed_at = Utc::now();
            db.insert_job(
                &job_id,
                JobStatus::Failed,
                Some(&format!("{err:#}")),
                effective.conversation_id.as_deref(),
                now,
                failed_at,
            )?;
            emit_job_state(
                stream,
                JobState {
                    job_id,
                    status: JobStatus::Failed,
                    message: Some(format!("{err:#}")),
                    conversation_id: effective.conversation_id,
                    updated_at: failed_at,
                    artifacts: vec![],
                },
            )?;
            Err(err)
        }
    }
}

fn handle_request(
    config: &AppConfig,
    db: &Database,
    browser: &mut BrowserManager,
    started_at: chrono::DateTime<Utc>,
    request: DaemonRequest,
) -> Result<DaemonResponse> {
    Ok(match request {
        DaemonRequest::Ping => DaemonResponse::Health(health_snapshot(config, browser.health()?)),
        DaemonRequest::Status => DaemonResponse::Status(status_snapshot(config, started_at)),
        DaemonRequest::Shutdown => DaemonResponse::Ack(AckEvent {
            accepted: true,
            message: "daemon stopping".to_owned(),
        }),
        DaemonRequest::Doctor => {
            let browser_snapshot = browser.profile_info()?;
            DaemonResponse::Doctor(DoctorReport {
                health: health_snapshot(config, browser_snapshot.clone()),
                status: status_snapshot(config, started_at),
                config: config.snapshot(),
                capabilities: browser_snapshot.capability_report.clone(),
                browser: browser_snapshot,
            })
        }
        DaemonRequest::Ask(_) => unreachable!("ask requests are handled as streaming responses"),
        DaemonRequest::Capabilities => DaemonResponse::Capabilities(CapabilitySnapshot {
            report: browser.capabilities()?,
        }),
        DaemonRequest::ConfigShow => DaemonResponse::Config(config.snapshot()),
        DaemonRequest::SessionsList => DaemonResponse::Sessions(db.list_sessions()?),
        DaemonRequest::JobsList => DaemonResponse::Jobs(db.list_jobs()?),
        DaemonRequest::BrowserOpen(request) => {
            let snapshot = browser.open(request)?;
            db.upsert_session(
                "browser-primary",
                "Managed browser",
                snapshot.current_url.as_deref(),
                Utc::now(),
            )?;
            DaemonResponse::Browser(snapshot)
        }
        DaemonRequest::BrowserClose => DaemonResponse::Browser(browser.close()?),
        DaemonRequest::BrowserResetProfile => DaemonResponse::Browser(browser.reset_profile()?),
        DaemonRequest::BrowserProfileInfo => DaemonResponse::Browser(browser.profile_info()?),
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

pub fn health_snapshot(
    config: &AppConfig,
    browser: shared_protocol::BrowserStatusSnapshot,
) -> HealthCheck {
    HealthCheck {
        status: HealthStatus::Ok,
        checked_at: Utc::now(),
        browser_connected: browser.running,
        extension_connected: browser.extension_connected,
        authenticated: browser.authenticated,
        details: Some(if config.warm_mode() {
            format!(
                "control plane ready; warm mode enabled; browser_running={}; extension_connected={}",
                browser.running, browser.extension_connected
            )
        } else {
            match browser.current_url {
                Some(url) => format!(
                    "control plane ready; managed tab at {url}; extension_connected={}",
                    browser.extension_connected
                ),
                None => "control plane ready; browser runtime not started yet".to_owned(),
            }
        }),
    }
}

pub fn map_error(err: anyhow::Error) -> DaemonResponse {
    let message = format!("{err:#}");
    DaemonResponse::Error(DaemonErrorResponse {
        error_code: if message.contains("capability missing") {
            ErrorCode::CapabilityMissing
        } else if message.contains("ui drift") {
            ErrorCode::UiDrift
        } else if message.contains("timed out") {
            ErrorCode::ActionTimeout
        } else {
            ErrorCode::Transport
        },
        message,
    })
}

fn emit_job_state(stream: &interprocess::local_socket::Stream, state: JobState) -> Result<()> {
    ipc::write_frame(stream, &DaemonFrame::Event(EventPayload::JobState(state)))
}
