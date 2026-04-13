use crate::config::AppConfig;
use crate::core::{CoreService, error_code_for_message};
use crate::ipc;
use crate::lockfile;
use crate::logging::JsonLogger;
use anyhow::Result;
use interprocess::local_socket::traits::Listener as LocalSocketListenerTrait;
use serde_json::json;
use shared_protocol::{
    DaemonErrorResponse, DaemonFrame, DaemonRequest, DaemonResponse, EventPayload,
};
use std::sync::Arc;
use std::thread;

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
    let listener = ipc::bind(&config.paths.ipc_endpoint)?;
    let core = Arc::new(CoreService::new(config.clone())?);

    if config.warm_mode()
        && let Err(err) = core.warm_browser_if_enabled()
    {
        logger.warn(
            "warm browser open failed",
            json!({
                "error": format!("{err:#}"),
            }),
        )?;
    }

    logger.info(
        "daemon ready",
        json!({
            "db_path": config.paths.db_path,
            "warm_mode": config.warm_mode(),
        }),
    )?;

    loop {
        let stream = listener.accept()?;
        let (request, stream) = ipc::read_request(stream)?;
        logger.info("daemon request", json!({ "request": request }))?;

        let should_shutdown = matches!(request, DaemonRequest::Shutdown);
        match request {
            DaemonRequest::Ask(command) => {
                let failure_mode = Some(command.mode);
                if let Err(err) = handle_ask_request(&core, &stream, command) {
                    let error_message = format!("{err:#}");
                    logger.warn(
                        "daemon request failed",
                        json!({
                            "error": error_message,
                        }),
                    )?;
                    if let Ok(bundle) =
                        core.capture_failure_bundle(failure_mode, &error_message, None)
                    {
                        let _ = ipc::write_frame(
                            &stream,
                            &DaemonFrame::Event(EventPayload::Failure(bundle)),
                        );
                    }
                    ipc::write_frame(&stream, &DaemonFrame::Response(map_error(err)))?;
                }
                continue;
            }
            DaemonRequest::Research(command) => {
                let core = Arc::clone(&core);
                let config = config.clone();
                thread::spawn(move || {
                    let logger = JsonLogger::new(&config.paths.log_path).ok();
                    if let Err(err) = handle_research_request(&core, &stream, command) {
                        let error_message = format!("{err:#}");
                        if let Ok(bundle) = core.capture_failure_bundle(
                            Some(shared_protocol::Mode::Research),
                            &error_message,
                            None,
                        ) {
                            let _ = ipc::write_frame(
                                &stream,
                                &DaemonFrame::Event(EventPayload::Failure(bundle)),
                            );
                        }
                        let _ = ipc::write_frame(&stream, &DaemonFrame::Response(map_error(err)));
                        if let Some(logger) = logger {
                            let _ = logger.warn(
                                "daemon research request failed",
                                json!({
                                    "error": error_message,
                                }),
                            );
                        }
                    }
                });
                continue;
            }
            other => {
                let response = match handle_request(&core, other) {
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
        }
    }

    Ok(())
}

fn handle_ask_request(
    core: &CoreService,
    stream: &interprocess::local_socket::Stream,
    command: shared_protocol::AskCommand,
) -> Result<()> {
    let result = core.ask(command, |event| {
        ipc::write_frame(stream, &DaemonFrame::Event(event.clone()))?;
        Ok(())
    })?;
    ipc::write_frame(stream, &DaemonFrame::Response(DaemonResponse::Ask(result)))
}

fn handle_research_request(
    core: &CoreService,
    stream: &interprocess::local_socket::Stream,
    command: shared_protocol::ResearchCommand,
) -> Result<()> {
    let result = core.research(command, |event| {
        ipc::write_frame(stream, &DaemonFrame::Event(event.clone()))?;
        Ok(())
    })?;
    ipc::write_frame(
        stream,
        &DaemonFrame::Response(DaemonResponse::Research(result)),
    )
}

fn handle_request(core: &CoreService, request: DaemonRequest) -> Result<DaemonResponse> {
    Ok(match request {
        DaemonRequest::Ping => DaemonResponse::Health(core.health_snapshot()?),
        DaemonRequest::Status => DaemonResponse::Status(core.status_snapshot()),
        DaemonRequest::Shutdown => DaemonResponse::Ack(shared_protocol::AckEvent {
            accepted: true,
            message: "daemon stopping".to_owned(),
        }),
        DaemonRequest::Doctor => DaemonResponse::Doctor(core.doctor_report()?),
        DaemonRequest::Ask(_) => unreachable!("ask requests are handled as streaming responses"),
        DaemonRequest::Research(_) => {
            unreachable!("research requests are handled as streaming responses")
        }
        DaemonRequest::Capabilities => DaemonResponse::Capabilities(core.capabilities()?),
        DaemonRequest::ConfigShow => DaemonResponse::Config(core.config_snapshot()),
        DaemonRequest::SessionsList => DaemonResponse::Sessions(core.list_sessions()?),
        DaemonRequest::JobsList => DaemonResponse::Jobs(core.list_jobs()?),
        DaemonRequest::JobsShow(job_id) => DaemonResponse::Job(core.job_detail(&job_id)?),
        DaemonRequest::JobsCancel(command) => DaemonResponse::Ack(core.cancel_job(command)?),
        DaemonRequest::ArtifactsList => DaemonResponse::Artifacts(core.list_artifacts()?),
        DaemonRequest::ArtifactPull(command) => {
            DaemonResponse::ArtifactPull(core.pull_artifact(command)?)
        }
        DaemonRequest::BrowserOpen(request) => DaemonResponse::Browser(core.browser_open(request)?),
        DaemonRequest::BrowserClose => DaemonResponse::Browser(core.browser_close()?),
        DaemonRequest::BrowserLogout => DaemonResponse::Browser(core.browser_logout()?),
        DaemonRequest::BrowserResetProfile => {
            DaemonResponse::Browser(core.browser_reset_profile()?)
        }
        DaemonRequest::BrowserProfileInfo => DaemonResponse::Browser(core.browser_profile_info()?),
    })
}

fn map_error(err: anyhow::Error) -> DaemonResponse {
    let message = format!("{err:#}");
    DaemonResponse::Error(DaemonErrorResponse {
        error_code: error_code_for_message(&message),
        message,
    })
}
