pub mod build_info;
mod capabilities;
mod chat;
mod openai;
mod process_control;
mod scheduler;
mod uploads;

use crate::capabilities::{CapabilityCatalog, CapabilityCatalogCache};
use crate::chat::{
    PreparedChatRequest, RequestOutcome, RequestOutcomeKind, build_request_summary,
    execute_chat_completions_request, outcome_from_error, prepare_chat_completions_request,
};
use crate::openai::{
    PROVIDER_MODELS, RequestRole, model_object, openai_error, provider_capability_catalog,
    request_role_name,
};
use crate::process_control::{
    GatewayRuntimeGuard, acquire_gateway_lock, gateway_log_path, gateway_status_path,
    write_status_snapshot,
};
use crate::scheduler::{CancelDisposition, GatewayScheduler, SchedulerSnapshot};
use crate::uploads::{StagedFileMetadata, UploadStore};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use gptd::core::error_code_for_message;
use gptd::logging::JsonLogger;
use gptd::{AppConfig, CoreService};
use serde_json::{Value, json};
use shared_protocol::{
    ArtifactSummary, AuthSource, BrowserConnectionMode, BrowserOpenRequest, BrowserOwner,
    BrowserStatusSnapshot, ConversationSelectCommand, ConversationSelection, ConversationSummary,
    DaemonRequest, DaemonResponse, DoctorReport, ErrorCode, GatewayRequestContext, HealthCheck,
    JobStatus,
};
use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(test)]
use crate::uploads::{gateway_uploads_dir, staged_metadata_path};
#[cfg(test)]
use shared_protocol::CapabilityReport;
#[cfg(test)]
use std::process::Command;

pub use crate::process_control::{
    GatewayStatusSnapshot, gateway_doctor, gateway_status, load_status_snapshot,
    start_gateway_process, stop_gateway_process,
};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const STARTUP_PROBE_TIMEOUT_MS: u64 = 1_000;
const HEALTH_PATH: &str = "/health";
const READY_PATH: &str = "/ready";
const ADMIN_STATUS_PATH: &str = "/admin/status";
const ADMIN_SHUTDOWN_PATH: &str = "/admin/shutdown";
const V1_MODELS_PATH: &str = "/v1/models";
const V1_CAPABILITIES_PATH: &str = "/v1/capabilities";
const V1_CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
const V1_FILES_PATH: &str = "/v1/files";
const V1_ARTIFACTS_PATH: &str = "/v1/artifacts";
const V1_PROVIDER_PATH: &str = "/v1/provider";
const V1_SESSIONS_PATH: &str = "/v1/sessions";
const V1_REQUESTS_PATH: &str = "/v1/requests";
const V1_CONVERSATIONS_PATH: &str = "/v1/conversations";
const V1_CONVERSATIONS_SELECT_PATH: &str = "/v1/conversations/select";
const V1_USAGE_PATH: &str = "/v1/usage";
const V1_REQUESTS_PREFIX: &str = "/v1/requests/";
const STAGED_UPLOAD_RETENTION_SECS: i64 = 24 * 60 * 60;
const REQUEST_SUMMARY_LIMIT: usize = 50;
const REQUEST_LIST_MAX_LIMIT: usize = 100;

#[derive(Debug)]
struct RequestContext {
    request_id: String,
    request: HttpRequest,
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Debug)]
struct HttpResponse {
    status_code: u16,
    reason_phrase: &'static str,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

#[derive(Debug)]
struct SimpleHttpResponse {
    status_code: u16,
    #[cfg(test)]
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
struct RequestListQuery {
    limit: usize,
    offset: usize,
    status: Option<JobStatus>,
}

fn daemon_health_snapshot(config: &AppConfig) -> Option<HealthCheck> {
    if !gptd::daemon_is_responsive(&config.paths.ipc_endpoint) {
        return None;
    }
    match gptd::send_request(&config.paths.ipc_endpoint, &DaemonRequest::Ping).ok()? {
        DaemonResponse::Health(health) => Some(health),
        _ => None,
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

fn effective_health_snapshot(config: &AppConfig, core: &CoreService) -> Result<HealthCheck> {
    if let Some(health) = daemon_health_snapshot(config) {
        Ok(health)
    } else {
        core.health_snapshot()
    }
}

fn effective_doctor_report(config: &AppConfig, core: &CoreService) -> Result<DoctorReport> {
    if let Some(report) = daemon_doctor_report(config) {
        Ok(report)
    } else {
        core.doctor_report()
    }
}

fn degraded_browser_status_snapshot(config: &AppConfig) -> BrowserStatusSnapshot {
    let attach_mode = config.browser_connection_mode();
    BrowserStatusSnapshot {
        running: false,
        browser_kind: if cfg!(windows) {
            "msedge".to_owned()
        } else {
            "chromium".to_owned()
        },
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
    }
}

pub fn run_gateway(config: AppConfig) -> Result<()> {
    let log_path = gateway_log_path(&config);
    let logger = Arc::new(JsonLogger::new(&log_path)?);
    let core = Arc::new(CoreService::new(config.clone())?);
    let capability_cache = Arc::new(CapabilityCatalogCache::load(&config)?);
    let upload_store = Arc::new(UploadStore::load(&config)?);
    let _lock = acquire_gateway_lock(&config)?;

    let listener =
        TcpListener::bind((config.gateway_host(), config.gateway_port())).with_context(|| {
            format!(
                "bind gateway {}:{}",
                config.gateway_host(),
                config.gateway_port()
            )
        })?;
    let actual_port = listener
        .local_addr()
        .context("resolve gateway local address")?
        .port();
    listener
        .set_nonblocking(true)
        .context("set gateway listener nonblocking")?;

    if config.warm_mode()
        && let Err(err) = core.warm_browser_if_enabled()
    {
        logger.warn(
            "gateway warm browser open failed",
            json!({
                "error": format!("{err:#}"),
            }),
        )?;
    }

    let snapshot = Arc::new(GatewayStatusSnapshot {
        pid: std::process::id(),
        started_at: Utc::now(),
        state_root: config.paths.state_root.display().to_string(),
        host: config.gateway_host().to_owned(),
        port: actual_port,
        api_key_present: config.gateway_api_key().is_some(),
        max_concurrent_upstream_jobs: config.gateway_max_concurrent_upstream_jobs(),
        queue_length: config.gateway_queue_length(),
        queue_timeout_ms: config.gateway_queue_timeout_ms(),
        per_client_limit: config.gateway_per_client_limit(),
        request_timeout_ms: config.gateway_request_timeout_ms(),
        ask_timeout_ms: config.ask_timeout_ms(),
        extended_thinking_timeout_ms: config.extended_thinking_timeout_ms(),
        research_timeout_ms: config.research_timeout_ms(),
        safe_mode_behavior: config.gateway_safe_mode_behavior().to_owned(),
        log_path: log_path.display().to_string(),
    });
    write_status_snapshot(&config, &snapshot)?;
    let _runtime_guard = GatewayRuntimeGuard {
        status_path: gateway_status_path(&config),
    };

    logger.info(
        "gateway ready",
        json!({
            "request_id": "bootstrap",
            "host": snapshot.host,
            "port": snapshot.port,
            "safe_mode_behavior": snapshot.safe_mode_behavior,
            "max_concurrent_upstream_jobs": snapshot.max_concurrent_upstream_jobs,
            "queue_length": snapshot.queue_length,
            "queue_timeout_ms": snapshot.queue_timeout_ms,
            "per_client_limit": snapshot.per_client_limit,
        }),
    )?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let request_counter = Arc::new(AtomicU64::new(0));
    let scheduler = Arc::new(GatewayScheduler::new(&config));

    while !shutdown.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, addr)) => {
                let logger = Arc::clone(&logger);
                let core = Arc::clone(&core);
                let snapshot = Arc::clone(&snapshot);
                let shutdown = Arc::clone(&shutdown);
                let request_counter = Arc::clone(&request_counter);
                let scheduler = Arc::clone(&scheduler);
                let capability_cache = Arc::clone(&capability_cache);
                let upload_store = Arc::clone(&upload_store);
                let config = config.clone();
                thread::spawn(move || {
                    if let Err(err) = handle_connection(
                        stream,
                        addr.to_string(),
                        &config,
                        &core,
                        &snapshot,
                        &shutdown,
                        &request_counter,
                        &scheduler,
                        &capability_cache,
                        &upload_store,
                        &logger,
                    ) {
                        let _ = logger.warn(
                            "gateway connection failed",
                            json!({
                                "error": format!("{err:#}"),
                            }),
                        );
                    }
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(err) => return Err(err).context("accept gateway connection"),
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn handle_connection(
    mut stream: TcpStream,
    remote_addr: String,
    config: &AppConfig,
    core: &CoreService,
    snapshot: &GatewayStatusSnapshot,
    shutdown: &AtomicBool,
    request_counter: &AtomicU64,
    scheduler: &GatewayScheduler,
    capability_cache: &CapabilityCatalogCache,
    upload_store: &UploadStore,
    logger: &JsonLogger,
) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_millis(
            config.gateway_request_timeout_ms(),
        )))
        .context("set gateway read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_millis(
            config.gateway_request_timeout_ms(),
        )))
        .context("set gateway write timeout")?;

    let context = match parse_http_request(&mut stream, request_counter) {
        Ok(context) => context,
        Err(err) => {
            let request_id = next_request_id(request_counter);
            let response = if format!("{err:#}").contains("upload exceeds 20 MiB limit") {
                json_response(
                    413,
                    json!({
                        "request_id": request_id,
                        "error": "file exceeds 20 MiB limit",
                    }),
                )
            } else {
                json_response(
                    400,
                    json!({
                        "error": "bad_request",
                    }),
                )
            };
            logger.warn(
                "gateway request parse failed",
                json!({
                    "request_id": request_id,
                    "error": format!("{err:#}"),
                }),
            )?;
            return write_http_response(&mut stream, &request_id, response);
        }
    };

    log_request(logger, &context.request_id, &context.request, &remote_addr)?;
    let scheduler_status = scheduler.snapshot()?;
    let request_path = normalized_request_path(&context.request.path);

    let response = match (context.request.method.as_str(), request_path) {
        ("GET", HEALTH_PATH) => {
            let health = effective_health_snapshot(config, core)?;
            json_response(
                200,
                json!({
                    "service": "tablinq-gateway",
                    "request_id": context.request_id,
                    "status": health.status,
                    "health": health,
                    "gateway": snapshot,
                    "scheduler": scheduler_status,
                }),
            )
        }
        ("GET", READY_PATH) => json_response(
            200,
            json!({
                "service": "tablinq-gateway",
                "request_id": context.request_id,
                "ready": true,
                "gateway": snapshot,
                "scheduler": scheduler_status,
            }),
        ),
        ("GET", ADMIN_STATUS_PATH) => match authorize_request(config, &context.request, false) {
            Ok(()) => {
                let health = effective_health_snapshot(config, core)?;
                let (browser, browser_warning) = match effective_doctor_report(config, core) {
                    Ok(doctor) => (doctor.browser, Value::Null),
                    Err(err) => (
                        degraded_browser_status_snapshot(config),
                        Value::String(format!("{err:#}")),
                    ),
                };
                json_response(
                    200,
                    json!({
                        "request_id": context.request_id,
                        "gateway": snapshot,
                        "scheduler": scheduler_status,
                        "health": health,
                        "browser": browser,
                        "browser_warning": browser_warning,
                    }),
                )
            }
            Err(response) => response,
        },
        ("POST", ADMIN_SHUTDOWN_PATH) => match authorize_request(config, &context.request, false) {
            Ok(()) => {
                shutdown.store(true, Ordering::SeqCst);
                json_response(
                    200,
                    json!({
                        "request_id": context.request_id,
                        "accepted": true,
                    }),
                )
            }
            Err(response) => response,
        },
        ("GET", V1_MODELS_PATH) => match authorize_request(config, &context.request, true) {
            Ok(()) => handle_models_request(core, capability_cache, snapshot, &context.request_id),
            Err(response) => response,
        },
        ("GET", V1_CAPABILITIES_PATH) => match authorize_request(config, &context.request, true) {
            Ok(()) => handle_capabilities_request(core, capability_cache, &context.request_id)?,
            Err(response) => response,
        },
        ("GET", V1_PROVIDER_PATH) => match authorize_request(config, &context.request, false) {
            Ok(()) => {
                handle_provider_request(core, capability_cache, snapshot, &context.request_id)
            }
            Err(response) => response,
        },
        ("GET", V1_USAGE_PATH) => match authorize_request(config, &context.request, false) {
            Ok(()) => handle_usage_request(core, &context.request_id)?,
            Err(response) => response,
        },
        ("GET", V1_SESSIONS_PATH) => match authorize_request(config, &context.request, false) {
            Ok(()) => handle_sessions_request(core, &context.request_id)?,
            Err(response) => response,
        },
        ("GET", V1_REQUESTS_PATH) => match authorize_request(config, &context.request, false) {
            Ok(()) => handle_requests_request(core, &context)?,
            Err(response) => response,
        },
        ("GET", V1_CONVERSATIONS_PATH) => {
            match authorize_request(config, &context.request, false) {
                Ok(()) => handle_conversations_request(core, &context.request_id)?,
                Err(response) => response,
            }
        }
        ("GET", _) if request_id_from_path(&context.request.path).is_some() => {
            match authorize_request(config, &context.request, false) {
                Ok(()) => handle_request_summary_request(core, &context)?,
                Err(response) => response,
            }
        }
        ("GET", V1_FILES_PATH) => match authorize_request(config, &context.request, false) {
            Ok(()) => handle_files_list_request(upload_store, &context.request_id)?,
            Err(response) => response,
        },
        ("POST", V1_FILES_PATH) => match authorize_request(config, &context.request, false) {
            Ok(()) => handle_file_upload_request(upload_store, &context)?,
            Err(response) => response,
        },
        ("GET", _)
            if context
                .request
                .path
                .starts_with(&format!("{V1_FILES_PATH}/")) =>
        {
            match authorize_request(config, &context.request, false) {
                Ok(()) => handle_file_detail_request(upload_store, &context)?,
                Err(response) => response,
            }
        }
        ("GET", V1_ARTIFACTS_PATH) => match authorize_request(config, &context.request, false) {
            Ok(()) => handle_artifacts_list_request(core, &context.request_id)?,
            Err(response) => response,
        },
        ("GET", _)
            if context
                .request
                .path
                .starts_with(&format!("{V1_ARTIFACTS_PATH}/")) =>
        {
            match authorize_request(config, &context.request, false) {
                Ok(()) => handle_artifact_request(core, &context)?,
                Err(response) => response,
            }
        }
        ("POST", _) if request_path.starts_with(V1_REQUESTS_PREFIX) => {
            if let Err(response) = authorize_request(config, &context.request, false) {
                response
            } else {
                handle_cancel_request(core, scheduler, &context)?
            }
        }
        ("POST", V1_CONVERSATIONS_SELECT_PATH) => {
            if let Err(response) = authorize_request(config, &context.request, false) {
                response
            } else {
                handle_conversation_select_request(core, &context)?
            }
        }
        ("POST", V1_CHAT_COMPLETIONS_PATH) => {
            if let Err(response) = authorize_request(config, &context.request, true) {
                response
            } else {
                let prepared = match prepare_chat_completions_request(core, upload_store, &context)
                {
                    Ok(prepared) => prepared,
                    Err(err) => {
                        let response = map_openai_error(&context.request_id, &err, None);
                        log_response(logger, &context.request_id, response.status_code)?;
                        return write_http_response(&mut stream, &context.request_id, response);
                    }
                };
                if let Some(response) =
                    request_role_policy_response(&context.request_id, prepared.request_role)
                {
                    log_response(logger, &context.request_id, response.status_code)?;
                    return write_http_response(&mut stream, &context.request_id, response);
                }
                let _ = upload_store.mark_linked_usage(
                    &prepared.file_ids,
                    &context.request_id,
                    prepared.conversation_id_hint(),
                );
                let request_started_at = Utc::now();
                let client_id = client_id_for_request(&context.request, &remote_addr);
                let queue_wait_started_at = Instant::now();
                let _lease =
                    match scheduler.acquire(&context.request_id, prepared.job_id(), &client_id) {
                        Ok(lease) => lease,
                        Err(err) => {
                            let queue_wait_ms = queue_wait_started_at.elapsed().as_millis() as u64;
                            if format!("{err:#}").contains("cancelled") {
                                let _ =
                                    core.mark_job_cancelled(prepared.job_id(), &format!("{err:#}"));
                            } else {
                                let _ = core.fail_job(prepared.job_id(), &format!("{err:#}"));
                            }
                            let response = queue_error_response(
                                &context.request_id,
                                &scheduler.snapshot().unwrap_or(scheduler_status),
                                &err,
                            );
                            let _ = core.record_request_summary(&build_request_summary(
                                &context,
                                &prepared,
                                request_started_at,
                                Utc::now(),
                                queue_wait_ms,
                                outcome_from_error(&err),
                            ));
                            log_response(logger, &context.request_id, response.status_code)?;
                            return write_http_response(&mut stream, &context.request_id, response);
                        }
                    };
                let queue_wait_ms = queue_wait_started_at.elapsed().as_millis() as u64;
                if core
                    .job_cancel_requested(prepared.job_id())
                    .unwrap_or(false)
                {
                    let _ = core.mark_job_cancelled(
                        prepared.job_id(),
                        "request cancelled before execution",
                    );
                    let response = cancellation_response(
                        &context.request_id,
                        prepared.job_id(),
                        "request cancelled before execution",
                    );
                    let _ = core.record_request_summary(&build_request_summary(
                        &context,
                        &prepared,
                        request_started_at,
                        Utc::now(),
                        queue_wait_ms,
                        RequestOutcome {
                            kind: RequestOutcomeKind::Cancelled,
                            conversation_id: prepared.conversation_id_hint().map(str::to_owned),
                            interrupted_at: None,
                            interruption_reason: None,
                            recovered_at: None,
                            recovery_reason: None,
                            failed_at: None,
                            failure_reason: None,
                        },
                    ));
                    log_response(logger, &context.request_id, response.status_code)?;
                    return write_http_response(&mut stream, &context.request_id, response);
                }
                match execute_chat_completions_request(
                    &mut stream,
                    core,
                    &context,
                    logger,
                    &prepared,
                    &client_id,
                ) {
                    Ok(outcome) => {
                        let _ = upload_store.mark_linked_usage(
                            &prepared.file_ids,
                            &context.request_id,
                            outcome.conversation_id.as_deref(),
                        );
                        let _ = core.record_request_summary(&build_request_summary(
                            &context,
                            &prepared,
                            request_started_at,
                            Utc::now(),
                            queue_wait_ms,
                            outcome,
                        ));
                    }
                    Err(err) => {
                        let _ = core.record_request_summary(&build_request_summary(
                            &context,
                            &prepared,
                            request_started_at,
                            Utc::now(),
                            queue_wait_ms,
                            outcome_from_error(&err),
                        ));
                        let failure_bundle_id = capture_gateway_failure_bundle(
                            core, &context, &prepared, &client_id, &err,
                        );
                        let response = map_openai_error(
                            &context.request_id,
                            &err,
                            failure_bundle_id.as_deref(),
                        );
                        log_response(logger, &context.request_id, response.status_code)?;
                        return write_http_response(&mut stream, &context.request_id, response);
                    }
                }
                return Ok(());
            }
        }
        _ => json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ),
    };

    log_response(logger, &context.request_id, response.status_code)?;
    write_http_response(&mut stream, &context.request_id, response)
}

fn handle_models_request(
    core: &CoreService,
    capability_cache: &CapabilityCatalogCache,
    snapshot: &GatewayStatusSnapshot,
    request_id: &str,
) -> HttpResponse {
    let catalog = capability_cache
        .resolve(core)
        .unwrap_or_else(|_| CapabilityCatalog {
            source: "unknown",
            cache_key: "unavailable".to_owned(),
            profile_dir: core
                .config()
                .paths
                .browser_profile_dir
                .display()
                .to_string(),
            workspace_key: core.config().browser_target_url().to_owned(),
            report: None,
        });
    let models = PROVIDER_MODELS
        .iter()
        .map(|spec| model_object(spec, catalog.report.as_ref(), catalog.source))
        .collect::<Vec<_>>();
    json_response(
        200,
        json!({
            "object": "list",
            "data": models,
            "request_id": request_id,
            "tablinq_capability_source": catalog.source,
            "tablinq_model_ids": PROVIDER_MODELS.iter().map(|spec| spec.id).collect::<Vec<_>>(),
            "gateway": {
                "host": snapshot.host,
                "port": snapshot.port,
            }
        }),
    )
}

fn handle_capabilities_request(
    core: &CoreService,
    capability_cache: &CapabilityCatalogCache,
    request_id: &str,
) -> Result<HttpResponse> {
    let catalog = capability_cache.resolve(core)?;
    let aliases = catalog
        .report
        .as_ref()
        .map(|report| report.aliases.clone())
        .unwrap_or_default();
    let models = PROVIDER_MODELS
        .iter()
        .map(|spec| model_object(spec, catalog.report.as_ref(), catalog.source))
        .collect::<Vec<_>>();

    Ok(json_response(
        200,
        json!({
            "object": "tablinq.capability_catalog",
            "request_id": request_id,
            "source": catalog.source,
            "cache_key": catalog.cache_key,
            "profile_dir": catalog.profile_dir,
            "workspace_key": catalog.workspace_key,
            "reported_at": catalog.report.as_ref().map(|report| report.reported_at),
            "selector_pack_id": catalog.report.as_ref().map(|report| report.selector_pack_id.clone()),
            "selector_pack_version": catalog.report.as_ref().map(|report| report.selector_pack_version.clone()),
            "aliases": aliases,
            "models": models,
            "provider": provider_capability_catalog(catalog.report.as_ref(), catalog.source),
        }),
    ))
}

fn handle_provider_request(
    core: &CoreService,
    capability_cache: &CapabilityCatalogCache,
    snapshot: &GatewayStatusSnapshot,
    request_id: &str,
) -> HttpResponse {
    let catalog = capability_cache
        .resolve(core)
        .unwrap_or_else(|_| CapabilityCatalog {
            source: "unknown",
            cache_key: "unavailable".to_owned(),
            profile_dir: core
                .config()
                .paths
                .browser_profile_dir
                .display()
                .to_string(),
            workspace_key: core.config().browser_target_url().to_owned(),
            report: None,
        });
    json_response(
        200,
        json!({
            "object": "tablinq.provider",
            "request_id": request_id,
            "provider": "tablinq",
            "openai_compatible": true,
            "base_url": format!("http://{}:{}/v1", snapshot.host, snapshot.port),
            "capability_source": catalog.source,
            "model_ids": PROVIDER_MODELS.iter().map(|spec| spec.id).collect::<Vec<_>>(),
            "provider_capabilities": provider_capability_catalog(catalog.report.as_ref(), catalog.source),
            "refusal_conditions": [
                {
                    "code": "auth_required",
                    "behavior": "refuse",
                    "when": "the managed browser session is signed out or has expired",
                },
                {
                    "code": "capability_missing",
                    "behavior": "refuse",
                    "when": "the selected live account or workspace no longer exposes the requested model or mode",
                },
                {
                    "code": "ui_drift",
                    "behavior": "refuse",
                    "when": "the live page no longer matches the known automation controls",
                },
                {
                    "code": "upstream_timeout",
                    "behavior": "refuse",
                    "when": "the managed page stalls instead of producing a stable streamed response",
                }
            ],
            "request_annotations": {
                "field": "tablinq_request_role",
                "parent_request_id_field": "tablinq_parent_request_id",
                "allowed_values": ["primary", "subagent", "branch"],
                "default": "primary",
                "policy": {
                    "primary": "execute",
                    "branch": "serialize",
                    "subagent": "reject",
                }
            },
            "presets": {
                "openclaude": {
                    "recommended_models": [
                        "tablinq-instant",
                        "tablinq-thinking-standard",
                        "tablinq-search",
                    ],
                    "streaming": true,
                    "max_parallel_requests": 1,
                    "subagents": "disabled_by_default",
                    "subagent_recommendation": "disable",
                },
                "paperclip": {
                    "transport": "openai_compatible_http",
                    "recommended_models": [
                        "tablinq-instant",
                        "tablinq-search",
                        "tablinq-research",
                    ],
                    "streaming": true,
                    "max_parallel_requests": 1,
                    "artifacts": true,
                    "files": true,
                    "usage_accounting": "unknown_or_estimated",
                    "subagent_recommendation": "disable",
                    "budget_guidance": {
                        "rate_limit_hint": "keep provider concurrency at 1",
                        "retry_backoff_hint": "respect queue_timeout and retry_after_ms on 429 responses",
                    }
                }
            }
        }),
    )
}

fn handle_usage_request(core: &CoreService, request_id: &str) -> Result<HttpResponse> {
    let history = core.list_request_summaries(REQUEST_SUMMARY_LIMIT)?;
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.request_summary_list",
            "request_id": request_id,
            "data": history.requests,
            "limit": REQUEST_SUMMARY_LIMIT,
            "warnings": ["token usage is unavailable for the browser-backed provider"],
        }),
    ))
}

fn handle_requests_request(core: &CoreService, context: &RequestContext) -> Result<HttpResponse> {
    let query = match parse_request_list_query(&context.request.path, &context.request_id) {
        Ok(query) => query,
        Err(response) => return Ok(response),
    };
    let page = core.list_request_page(query.status, query.limit, query.offset)?;
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.request_list",
            "request_id": context.request_id,
            "data": page.requests,
            "pagination": {
                "limit": page.limit,
                "offset": page.offset,
                "next_offset": page.next_offset,
                "has_more": page.next_offset.is_some(),
            },
            "filter": {
                "status": page.status,
            },
        }),
    ))
}

fn handle_sessions_request(core: &CoreService, request_id: &str) -> Result<HttpResponse> {
    let sessions = core.list_sessions()?;
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.session_list",
            "request_id": request_id,
            "data": sessions.sessions,
        }),
    ))
}

fn handle_conversations_request(core: &CoreService, request_id: &str) -> Result<HttpResponse> {
    let conversations = core.list_conversations()?;
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.conversation_list",
            "request_id": request_id,
            "data": conversations
                .conversations
                .into_iter()
                .map(conversation_summary_object)
                .collect::<Vec<_>>(),
        }),
    ))
}

fn handle_conversation_select_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let command = match serde_json::from_slice::<ConversationSelectCommand>(&context.request.body) {
        Ok(command) => command,
        Err(err) => {
            return Ok(json_response(
                400,
                json!({
                    "request_id": context.request_id,
                    "error": "invalid_request",
                    "message": format!("invalid JSON body: {err}"),
                }),
            ));
        }
    };
    if let Err(err) = command.validate() {
        return Ok(json_response(
            400,
            json!({
                "request_id": context.request_id,
                "error": "invalid_request",
                "message": format!("{err}"),
            }),
        ));
    }
    if let Some(conversation_id) = command.conversation_id.as_deref() {
        if core.find_conversation(conversation_id)?.is_none() {
            return Ok(json_response(
                404,
                json!({
                    "request_id": context.request_id,
                    "error": "conversation_not_found",
                    "message": format!("conversation {conversation_id} was not found"),
                }),
            ));
        }
    }

    match core.select_conversation(command) {
        Ok(selection) => Ok(json_response(
            200,
            json!({
                "object": "tablinq.conversation_selection",
                "request_id": context.request_id,
                "data": conversation_selection_object(selection),
            }),
        )),
        Err(err) => {
            let message = format!("{err:#}");
            let (status, error) =
                if message.contains("was not found") && message.contains("conversation") {
                    (404, "conversation_not_found")
                } else {
                    (400, "invalid_request")
                };
            Ok(json_response(
                status,
                json!({
                    "request_id": context.request_id,
                    "error": error,
                    "message": message,
                }),
            ))
        }
    }
}

fn conversation_summary_object(summary: ConversationSummary) -> Value {
    let conversation_id = summary.conversation_id;
    json!({
        "conversation_id": conversation_id.clone(),
        "tablinq_conversation_id": conversation_id,
        "session_id": summary.session_id,
        "label": summary.label,
        "tab_url": summary.tab_url,
        "last_prompt": summary.last_prompt,
        "last_response": summary.last_response,
        "last_seen_at": summary.last_seen_at,
    })
}

fn conversation_selection_object(selection: ConversationSelection) -> Value {
    let conversation_id = selection.conversation_id.clone();
    json!({
        "conversation_id": conversation_id.clone(),
        "tablinq_conversation_id": selection.conversation_id,
        "session_id": selection.session_id,
        "current_url": selection.current_url,
        "browser": selection.browser,
    })
}

fn handle_request_summary_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(target_request_id) = request_id_from_path(&context.request.path) else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };

    let summary = match core.request_summary(target_request_id) {
        Ok(summary) => summary,
        Err(_) => {
            return Ok(json_response(
                404,
                json!({
                    "request_id": context.request_id,
                    "error": "not_found",
                }),
            ));
        }
    };

    Ok(json_response(
        200,
        json!({
            "object": "tablinq.request_summary",
            "request_id": context.request_id,
            "data": summary,
        }),
    ))
}

fn handle_files_list_request(upload_store: &UploadStore, request_id: &str) -> Result<HttpResponse> {
    let files = upload_store
        .list()?
        .into_iter()
        .map(staged_file_object)
        .collect::<Vec<_>>();
    Ok(json_response(
        200,
        json!({
            "object": "list",
            "request_id": request_id,
            "data": files,
        }),
    ))
}

fn handle_file_upload_request(
    upload_store: &UploadStore,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(file_name) = context
        .request
        .headers
        .get("x-file-name")
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(json_response(
            400,
            json!({
                "request_id": context.request_id,
                "error": "x-file-name header is required",
            }),
        ));
    };
    if context.request.body.len() > crate::uploads::MAX_STAGED_UPLOAD_BYTES {
        return Ok(json_response(
            413,
            json!({
                "request_id": context.request_id,
                "error": "file exceeds 20 MiB limit",
            }),
        ));
    }
    let metadata = upload_store.stage_upload(
        file_name,
        context
            .request
            .headers
            .get("content-type")
            .map(String::as_str),
        &context.request.body,
    )?;
    Ok(json_response(
        201,
        json!({
            "request_id": context.request_id,
            "data": staged_file_object(metadata),
        }),
    ))
}

fn handle_file_detail_request(
    upload_store: &UploadStore,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(file_id) = file_id_from_path(&context.request.path) else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };
    let Some(metadata) = upload_store.get(file_id)? else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };
    Ok(json_response(
        200,
        json!({
            "request_id": context.request_id,
            "data": staged_file_object(metadata),
        }),
    ))
}

fn handle_artifacts_list_request(core: &CoreService, request_id: &str) -> Result<HttpResponse> {
    let artifacts = core
        .list_artifacts()?
        .artifacts
        .into_iter()
        .map(|artifact| artifact_object(core, artifact))
        .collect::<Result<Vec<_>>>()?;
    Ok(json_response(
        200,
        json!({
            "object": "list",
            "request_id": request_id,
            "data": artifacts,
        }),
    ))
}

fn handle_artifact_request(core: &CoreService, context: &RequestContext) -> Result<HttpResponse> {
    let Some((artifact_id, wants_content)) = artifact_id_from_path(&context.request.path) else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };
    let artifact = core
        .list_artifacts()?
        .artifacts
        .into_iter()
        .find(|item| item.artifact_id == artifact_id);
    let Some(artifact) = artifact else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };

    if wants_content {
        let body = fs::read(&artifact.storage_path)
            .with_context(|| format!("read {}", artifact.storage_path))?;
        return Ok(HttpResponse {
            status_code: 200,
            reason_phrase: reason_phrase_for_status(200),
            headers: vec![
                ("Content-Type".to_owned(), artifact.content_type.clone()),
                (
                    "Content-Disposition".to_owned(),
                    format!("attachment; filename=\"{}\"", artifact.file_name),
                ),
            ],
            body,
        });
    }

    Ok(json_response(
        200,
        json!({
            "request_id": context.request_id,
            "data": artifact_object(core, artifact)?,
        }),
    ))
}

fn staged_file_object(metadata: StagedFileMetadata) -> Value {
    json!({
        "id": metadata.file_id,
        "object": "tablinq.file",
        "file_name": metadata.file_name,
        "content_type": metadata.content_type,
        "size_bytes": metadata.size_bytes,
        "sha256": metadata.sha256,
        "created_at": metadata.created_at,
        "expires_at": metadata.expires_at,
        "linked_request_ids": metadata.linked_request_ids,
        "linked_conversation_ids": metadata.linked_conversation_ids,
    })
}

fn artifact_object(core: &CoreService, artifact: ArtifactSummary) -> Result<Value> {
    let linked_job_ids = core.artifact_job_ids(&artifact.artifact_id)?;
    let linked_request_ids = core.artifact_request_ids(&artifact.artifact_id)?;
    Ok(json!({
        "id": artifact.artifact_id,
        "object": "tablinq.artifact",
        "file_name": artifact.file_name,
        "content_type": artifact.content_type,
        "size_bytes": artifact.size_bytes,
        "sha256": artifact.sha256,
        "conversation_id": artifact.conversation_id,
        "message_id": artifact.message_id,
        "created_at": artifact.created_at,
        "linked_job_ids": linked_job_ids,
        "linked_request_ids": linked_request_ids,
        "content_url": format!("{V1_ARTIFACTS_PATH}/{}/content", artifact.artifact_id),
    }))
}

fn request_role_policy_response(request_id: &str, role: RequestRole) -> Option<HttpResponse> {
    match role {
        RequestRole::Primary | RequestRole::Branch => None,
        RequestRole::Subagent => {
            let mut body = openai_error(
                429,
                "overloaded_error",
                "subagents_disabled",
                &format!(
                    "{request_id}: subagent requests are disabled by default for the single-browser OpenClaude preset"
                ),
            );
            body["request_id"] = Value::String(request_id.to_owned());
            body["tablinq_refusal_reason"] = Value::String("subagents_disabled".to_owned());
            body["tablinq_guidance"] = json!(provider_guidance_for_code("subagents_disabled"));
            Some(json_response(429, body))
        }
    }
}

fn capture_gateway_failure_bundle(
    core: &CoreService,
    context: &RequestContext,
    prepared: &PreparedChatRequest,
    client_id: &str,
    err: &anyhow::Error,
) -> Option<String> {
    if !should_capture_gateway_failure_bundle(err) {
        return None;
    }
    let message = format!("{err:#}");
    core.capture_failure_bundle(
        Some(prepared.mode()),
        &message,
        Some(GatewayRequestContext {
            request_id: context.request_id.clone(),
            request_path: context.request.path.clone(),
            client_id: Some(client_id.to_owned()),
            model: Some(prepared.model_id.to_owned()),
            request_role: Some(request_role_name(prepared.request_role).to_owned()),
            parent_request_id: prepared.parent_request_id.clone(),
            job_id: Some(prepared.job_id().to_owned()),
            streaming: prepared.request.stream,
        }),
    )
    .ok()
    .map(|bundle| bundle.bundle_id)
}

fn should_capture_gateway_failure_bundle(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}");
    matches!(
        error_code_for_message(&message),
        ErrorCode::AuthRequired
            | ErrorCode::CapabilityMissing
            | ErrorCode::BrowserLaunch
            | ErrorCode::ProfileLocked
            | ErrorCode::ActionTimeout
            | ErrorCode::UiDrift
            | ErrorCode::ResearchJobFailed
    )
}

fn provider_guidance_for_code(code: &str) -> Vec<&'static str> {
    match code {
        "auth_required" => vec![
            "provider mode refuses requests when the managed browser is signed out; sign in in the managed profile and retry",
            "run gpt doctor or GET /ready to confirm the browser session is authenticated before retrying queued work",
        ],
        "capability_missing" => vec![
            "provider mode refuses requests when the selected model or mode is unavailable instead of guessing a fallback",
            "refresh GET /v1/capabilities after account, workspace, or UI changes before retrying the request",
        ],
        "browser_unavailable" => vec![
            "provider mode refuses requests until the managed browser can be opened exclusively",
            "close other managed-profile browser instances or reset the managed profile if the browser stays unavailable",
        ],
        "upstream_timeout" => vec![
            "provider mode refuses long-stalled requests instead of inventing a partial answer",
            "inspect the latest failure bundle and retry after the managed tab is responsive again",
        ],
        "ui_drift" => vec![
            "provider mode refuses when the live page no longer matches known controls",
            "inspect the linked failure bundle before retrying so selector or capability drift is actionable",
        ],
        "research_job_failed" => vec![
            "provider mode refuses failed research runs instead of returning incomplete research output",
        ],
        "subagents_disabled" => vec![
            "provider mode keeps subagents disabled by default to protect the single warm browser session",
        ],
        _ => Vec::new(),
    }
}

fn refusal_reason_for_code(code: &str) -> Option<&str> {
    match code {
        "auth_required"
        | "capability_missing"
        | "browser_unavailable"
        | "upstream_timeout"
        | "ui_drift"
        | "research_job_failed"
        | "subagents_disabled" => Some(code),
        _ => None,
    }
}

fn parse_request_list_query(
    path: &str,
    request_id: &str,
) -> std::result::Result<RequestListQuery, HttpResponse> {
    let mut query = RequestListQuery {
        limit: REQUEST_SUMMARY_LIMIT,
        offset: 0,
        status: None,
    };
    for (key, value) in request_query_pairs(path) {
        match key {
            "limit" => {
                let parsed = value.parse::<usize>().map_err(|_| {
                    json_response(
                        400,
                        json!({
                            "request_id": request_id,
                            "error": "invalid_request",
                            "message": "limit must be a positive integer",
                        }),
                    )
                })?;
                if parsed == 0 || parsed > REQUEST_LIST_MAX_LIMIT {
                    return Err(json_response(
                        400,
                        json!({
                            "request_id": request_id,
                            "error": "invalid_request",
                            "message": format!("limit must be between 1 and {REQUEST_LIST_MAX_LIMIT}"),
                        }),
                    ));
                }
                query.limit = parsed;
            }
            "offset" => {
                query.offset = value.parse::<usize>().map_err(|_| {
                    json_response(
                        400,
                        json!({
                            "request_id": request_id,
                            "error": "invalid_request",
                            "message": "offset must be a non-negative integer",
                        }),
                    )
                })?;
            }
            "status" => {
                query.status = Some(value.parse::<JobStatus>().map_err(|_| {
                    json_response(
                        400,
                        json!({
                            "request_id": request_id,
                            "error": "invalid_request",
                            "message": format!("unsupported status filter: {value}"),
                        }),
                    )
                })?);
            }
            _ => {}
        }
    }
    Ok(query)
}

fn normalized_request_path(path: &str) -> &str {
    path.split_once('?').map(|(base, _)| base).unwrap_or(path)
}

fn request_query_pairs(path: &str) -> impl Iterator<Item = (&str, &str)> {
    path.split_once('?')
        .map(|(_, query)| query)
        .into_iter()
        .flat_map(|query| query.split('&'))
        .filter_map(|pair| {
            if pair.is_empty() {
                None
            } else if let Some((key, value)) = pair.split_once('=') {
                Some((key, value))
            } else {
                Some((pair, ""))
            }
        })
}

fn file_id_from_path(path: &str) -> Option<&str> {
    normalized_request_path(path)
        .strip_prefix(&format!("{V1_FILES_PATH}/"))
        .filter(|value| !value.contains('/'))
}

fn artifact_id_from_path(path: &str) -> Option<(&str, bool)> {
    let suffix = normalized_request_path(path).strip_prefix(&format!("{V1_ARTIFACTS_PATH}/"))?;
    if let Some(artifact_id) = suffix.strip_suffix("/content") {
        if artifact_id.contains('/') || artifact_id.is_empty() {
            return None;
        }
        Some((artifact_id, true))
    } else if suffix.contains('/') || suffix.is_empty() {
        None
    } else {
        Some((suffix, false))
    }
}

fn handle_cancel_request(
    core: &CoreService,
    scheduler: &GatewayScheduler,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(request_id) = cancel_request_id_from_path(&context.request.path) else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };

    let Some((job_id, disposition)) = scheduler.cancel(request_id)? else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };

    let ack = core.cancel_job(shared_protocol::CancelJobCommand {
        job_id: job_id.clone(),
    })?;
    Ok(json_response(
        200,
        json!({
            "request_id": context.request_id,
            "target_request_id": request_id,
            "job_id": job_id,
            "accepted": ack.accepted,
            "message": ack.message,
            "state": match disposition {
                CancelDisposition::Queued => "queued",
                CancelDisposition::Running => "running",
            },
        }),
    ))
}

fn cancel_request_id_from_path(path: &str) -> Option<&str> {
    let suffix = normalized_request_path(path).strip_prefix(V1_REQUESTS_PREFIX)?;
    suffix.strip_suffix("/cancel")
}

fn request_id_from_path(path: &str) -> Option<&str> {
    let suffix = normalized_request_path(path).strip_prefix(V1_REQUESTS_PREFIX)?;
    if suffix.is_empty() || suffix.contains('/') {
        None
    } else {
        Some(suffix)
    }
}

fn client_id_for_request(request: &HttpRequest, remote_addr: &str) -> String {
    request
        .headers
        .get("x-client-id")
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            remote_addr
                .rsplit_once(':')
                .map(|(host, _)| host.to_owned())
                .unwrap_or_else(|| remote_addr.to_owned())
        })
}

fn queue_error_response(
    request_id: &str,
    scheduler: &SchedulerSnapshot,
    err: &anyhow::Error,
) -> HttpResponse {
    let message = format!("{err:#}");
    let (status, error_type, code) = if message.contains("queue timeout") {
        (429, "overloaded_error", "queue_timeout")
    } else if message.contains("per-client queue limit") {
        (429, "overloaded_error", "client_limit_exceeded")
    } else if message.contains("cancelled") {
        (409, "invalid_request_error", "cancelled")
    } else {
        (429, "overloaded_error", "queue_full")
    };
    let body = json!({
        "error": {
            "message": message,
            "type": error_type,
            "param": Value::Null,
            "code": code,
        },
        "request_id": request_id,
        "retry_after_ms": scheduler.queue_timeout_ms,
        "scheduler": scheduler,
    });
    HttpResponse {
        status_code: status,
        reason_phrase: reason_phrase_for_status(status),
        headers: vec![
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Retry-After".to_owned(), "1".to_owned()),
        ],
        body: serde_json::to_vec(&body).unwrap_or_default(),
    }
}

fn cancellation_response(request_id: &str, job_id: &str, message: &str) -> HttpResponse {
    json_response(
        409,
        json!({
            "error": {
                "message": message,
                "type": "invalid_request_error",
                "param": Value::Null,
                "code": "cancelled",
            },
            "request_id": request_id,
            "job_id": job_id,
            "cancelled": true,
            "job_status": "cancelled",
        }),
    )
}

fn ensure_browser_session(core: &CoreService) -> Result<()> {
    let mut snapshot = core.browser_profile_info()?;
    if !snapshot.running {
        core.browser_open(BrowserOpenRequest {
            target_url: None,
            browser_path: None,
            headless: Some(true),
        })?;
        snapshot = core.browser_profile_info()?;
    }
    if !snapshot.authenticated {
        anyhow::bail!("auth required: managed browser session is not authenticated");
    }
    Ok(())
}

fn discover_capabilities(core: &CoreService) -> Option<shared_protocol::CapabilityReport> {
    let needs_browser = match core.browser_profile_info() {
        Ok(snapshot) => !snapshot.running,
        Err(_) => true,
    };
    if needs_browser {
        let _ = core.browser_open(BrowserOpenRequest {
            target_url: None,
            browser_path: None,
            headless: Some(true),
        });
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_report = None;
    loop {
        if let Ok(snapshot) = core.capabilities()
            && let Some(report) = snapshot.report
        {
            if capability_report_is_usable(&report) {
                return Some(report);
            }
            last_report = Some(report);
        }

        if Instant::now() >= deadline {
            return last_report;
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn capability_report_is_usable(report: &shared_protocol::CapabilityReport) -> bool {
    let advertises_search = report.aliases.modes.contains_key("search");
    let advertises_research = report.aliases.modes.contains_key("research");
    let advertises_thinking = report.aliases.model_classes.contains_key("thinking");

    report.matrix.composer_ready
        && !report.aliases.modes.is_empty()
        && !report.aliases.model_classes.is_empty()
        && (!advertises_search || report.matrix.search_mode_visible)
        && (!advertises_research || report.matrix.research_mode_visible)
        && (!advertises_thinking
            || report.matrix.model_picker_visible
            || report.matrix.thinking_effort_visible)
        && report.matrix.upload_control_visible
}

fn authorize_request(
    config: &AppConfig,
    request: &HttpRequest,
    openai_shape: bool,
) -> std::result::Result<(), HttpResponse> {
    let Some(expected_api_key) = config.gateway_api_key() else {
        return Ok(());
    };
    let expected = format!("Bearer {expected_api_key}");
    let provided = request.headers.get("authorization").map(String::as_str);
    if provided == Some(expected.as_str()) {
        Ok(())
    } else if openai_shape {
        Err(json_response(
            401,
            openai_error(
                401,
                "invalid_request_error",
                "invalid_api_key",
                "missing or invalid gateway bearer token",
            ),
        ))
    } else {
        Err(json_response(
            401,
            json!({
                "error": "unauthorized",
            }),
        ))
    }
}

fn map_openai_error(
    request_id: &str,
    err: &anyhow::Error,
    failure_bundle_id: Option<&str>,
) -> HttpResponse {
    let (status, error_type, code, message) = map_openai_error_parts(err);
    json_response(
        status,
        annotate_openai_error_value(
            request_id,
            openai_error(
                status,
                error_type,
                code,
                &format!("{request_id}: {message}"),
            ),
            code,
            failure_bundle_id,
        ),
    )
}

fn map_openai_error_value(
    request_id: &str,
    err: &anyhow::Error,
    failure_bundle_id: Option<&str>,
) -> Value {
    let (status, error_type, code, message) = map_openai_error_parts(err);
    annotate_openai_error_value(
        request_id,
        openai_error(status, error_type, code, &message),
        code,
        failure_bundle_id,
    )
}

fn annotate_openai_error_value(
    request_id: &str,
    mut value: Value,
    code: &str,
    failure_bundle_id: Option<&str>,
) -> Value {
    value["request_id"] = Value::String(request_id.to_owned());
    if let Some(bundle_id) = failure_bundle_id {
        value["tablinq_failure_bundle_id"] = Value::String(bundle_id.to_owned());
    }
    let guidance = provider_guidance_for_code(code);
    if !guidance.is_empty() {
        value["tablinq_guidance"] = json!(guidance);
    }
    if let Some(reason) = refusal_reason_for_code(code) {
        value["tablinq_refusal_reason"] = Value::String(reason.to_owned());
    }
    value
}

fn map_openai_error_parts(err: &anyhow::Error) -> (u16, &'static str, &'static str, String) {
    let message = format!("{err:#}");
    if message.contains("cancelled") {
        return (409, "invalid_request_error", "cancelled", message);
    }
    if message.contains("was not found") && message.contains("conversation") {
        return (
            404,
            "invalid_request_error",
            "conversation_not_found",
            message,
        );
    }
    if message.contains("staged file") {
        return (400, "invalid_request_error", "file_not_found", message);
    }
    match error_code_for_message(&message) {
        ErrorCode::CapabilityMissing => {
            (400, "invalid_request_error", "capability_missing", message)
        }
        ErrorCode::AuthRequired => (503, "authentication_error", "auth_required", message),
        ErrorCode::BrowserLaunch | ErrorCode::ProfileLocked => (
            503,
            "service_unavailable_error",
            "browser_unavailable",
            message,
        ),
        ErrorCode::ActionTimeout => (504, "timeout_error", "upstream_timeout", message),
        ErrorCode::UiDrift => (502, "provider_error", "ui_drift", message),
        ErrorCode::ResearchJobFailed => (502, "provider_error", "research_job_failed", message),
        ErrorCode::UploadFailed => (400, "invalid_request_error", "upload_failed", message),
        ErrorCode::DownloadFailed => (502, "provider_error", "download_failed", message),
        ErrorCode::Transport => {
            if message.contains("queue full") {
                (429, "overloaded_error", "queue_full", message)
            } else if message.contains("queue timeout") {
                (429, "overloaded_error", "queue_timeout", message)
            } else if message.contains("per-client queue limit") {
                (429, "overloaded_error", "client_limit_exceeded", message)
            } else {
                (500, "server_error", "transport_error", message)
            }
        }
    }
}

fn is_client_disconnect_error(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}").to_ascii_lowercase();
    message.contains("broken pipe")
        || message.contains("connection reset")
        || message.contains("connection abort")
        || message.contains("forcibly closed")
        || message.contains("os error 10053")
}

fn write_sse_headers(stream: &mut TcpStream, request_id: &str) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\nX-Request-Id: {request_id}\r\n\r\n"
    )
    .context("write SSE headers")?;
    stream.flush().context("flush SSE headers")
}

fn write_sse_event(stream: &mut TcpStream, value: &Value) -> Result<()> {
    write!(
        stream,
        "data: {}\n\n",
        serde_json::to_string(value).context("encode SSE event")?
    )
    .context("write SSE event")?;
    stream.flush().context("flush SSE event")
}

fn write_sse_done(stream: &mut TcpStream) -> Result<()> {
    write!(stream, "data: [DONE]\n\n").context("write SSE done event")?;
    stream.flush().context("flush SSE done event")
}

fn log_request(
    logger: &JsonLogger,
    request_id: &str,
    request: &HttpRequest,
    remote_addr: &str,
) -> Result<()> {
    logger.info(
        "gateway request",
        json!({
            "request_id": request_id,
            "method": request.method,
            "path": request.path,
            "remote_addr": remote_addr,
            "body_len": request.body.len(),
        }),
    )
}

fn log_response(logger: &JsonLogger, request_id: &str, status_code: u16) -> Result<()> {
    logger.info(
        "gateway response",
        json!({
            "request_id": request_id,
            "status_code": status_code,
        }),
    )
}

fn json_response(status_code: u16, body: Value) -> HttpResponse {
    let body = serde_json::to_vec(&body).expect("serialize JSON response");
    HttpResponse {
        status_code,
        reason_phrase: reason_phrase_for_status(status_code),
        headers: vec![
            ("content-type".to_owned(), "application/json".to_owned()),
            ("connection".to_owned(), "close".to_owned()),
        ],
        body,
    }
}

fn reason_phrase_for_status(status_code: u16) -> &'static str {
    match status_code {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        409 => "Conflict",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Internal Server Error",
    }
}

fn parse_http_request(stream: &mut TcpStream, counter: &AtomicU64) -> Result<RequestContext> {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader
        .read_line(&mut request_line)
        .context("read request line")?;
    if request_line.trim().is_empty() {
        bail!("missing HTTP request line");
    }

    let mut request_line_parts = request_line.split_whitespace();
    let method = request_line_parts
        .next()
        .context("missing HTTP method")?
        .to_ascii_uppercase();
    let path = request_line_parts
        .next()
        .context("missing HTTP path")?
        .to_owned();

    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .context("read HTTP header line")?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let (name, value) = trimmed
            .split_once(':')
            .context("invalid HTTP header line")?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }

    let content_length = headers
        .get("content-length")
        .map(|value| value.parse::<usize>().context("parse Content-Length"))
        .transpose()?
        .unwrap_or(0);
    if method == "POST"
        && path == V1_FILES_PATH
        && content_length > crate::uploads::MAX_STAGED_UPLOAD_BYTES
    {
        bail!("upload exceeds 20 MiB limit");
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader
            .read_exact(&mut body)
            .context("read HTTP request body")?;
    }

    let request_id = headers
        .get("x-request-id")
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| next_request_id(counter));

    Ok(RequestContext {
        request_id,
        request: HttpRequest {
            method,
            path,
            headers,
            body,
        },
    })
}

fn write_http_response(
    stream: &mut TcpStream,
    request_id: &str,
    mut response: HttpResponse,
) -> Result<()> {
    response
        .headers
        .push(("x-request-id".to_owned(), request_id.to_owned()));
    response
        .headers
        .push(("content-length".to_owned(), response.body.len().to_string()));

    write!(
        stream,
        "HTTP/1.1 {} {}\r\n",
        response.status_code, response.reason_phrase
    )
    .context("write HTTP status line")?;
    for (name, value) in &response.headers {
        write!(stream, "{name}: {value}\r\n").context("write HTTP header")?;
    }
    write!(stream, "\r\n").context("write HTTP header terminator")?;
    stream
        .write_all(&response.body)
        .context("write HTTP response body")?;
    stream.flush().context("flush HTTP response")
}

fn next_request_id(counter: &AtomicU64) -> String {
    let seq = counter.fetch_add(1, Ordering::SeqCst) + 1;
    format!("req-{}-{seq}", Utc::now().timestamp_millis())
}

fn http_request(
    host: &str,
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    timeout_ms: u64,
) -> Result<SimpleHttpResponse> {
    let mut stream = TcpStream::connect((host, port))
        .with_context(|| format!("connect to gateway {host}:{port}"))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(timeout_ms)))
        .context("set gateway client read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_millis(timeout_ms)))
        .context("set gateway client write timeout")?;

    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    )
    .context("write gateway request line")?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").context("write gateway request header")?;
    }
    write!(stream, "\r\n").context("write gateway request terminator")?;
    if !body.is_empty() {
        stream
            .write_all(body)
            .context("write gateway request body")?;
    }
    stream.flush().context("flush gateway request")?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .context("read gateway status line")?;
    let parts: Vec<_> = status_line.split_whitespace().collect();
    if parts.len() < 2 {
        bail!("invalid gateway HTTP status line");
    }
    let status_code = parts[1]
        .parse::<u16>()
        .context("parse gateway status code")?;

    let mut response_headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .context("read gateway response header")?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let (name, value) = trimmed
            .split_once(':')
            .context("invalid gateway response header")?;
        response_headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }

    let content_length = response_headers
        .get("content-length")
        .map(|value| {
            value
                .parse::<usize>()
                .context("parse response Content-Length")
        })
        .transpose()?
        .unwrap_or(0);
    let mut response_body = vec![0u8; content_length];
    if content_length > 0 {
        reader
            .read_exact(&mut response_body)
            .context("read gateway response body")?;
    } else {
        reader
            .read_to_end(&mut response_body)
            .context("read gateway response body to end")?;
    }

    Ok(SimpleHttpResponse {
        status_code,
        #[cfg(test)]
        headers: response_headers,
        body: response_body,
    })
}

#[cfg(test)]
fn http_request_with_declared_content_length(
    host: &str,
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    declared_content_length: usize,
    timeout_ms: u64,
) -> Result<SimpleHttpResponse> {
    let mut stream = TcpStream::connect((host, port))
        .with_context(|| format!("connect to gateway {host}:{port}"))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(timeout_ms)))
        .context("set gateway client read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_millis(timeout_ms)))
        .context("set gateway client write timeout")?;

    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Length: {declared_content_length}\r\nConnection: close\r\n"
    )
    .context("write gateway request line")?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").context("write gateway request header")?;
    }
    write!(stream, "\r\n").context("write gateway request terminator")?;
    stream.flush().context("flush gateway request")?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .context("read gateway status line")?;
    let parts: Vec<_> = status_line.split_whitespace().collect();
    if parts.len() < 2 {
        bail!("invalid gateway HTTP status line");
    }
    let status_code = parts[1]
        .parse::<u16>()
        .context("parse gateway status code")?;

    let mut response_headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .context("read gateway response header")?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let (name, value) = trimmed
            .split_once(':')
            .context("invalid gateway response header")?;
        response_headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }

    let content_length = response_headers
        .get("content-length")
        .map(|value| {
            value
                .parse::<usize>()
                .context("parse response Content-Length")
        })
        .transpose()?
        .unwrap_or(0);
    let mut response_body = vec![0u8; content_length];
    if content_length > 0 {
        reader
            .read_exact(&mut response_body)
            .context("read gateway response body")?;
    } else {
        reader
            .read_to_end(&mut response_body)
            .context("read gateway response body to end")?;
    }

    Ok(SimpleHttpResponse {
        status_code,
        #[cfg(test)]
        headers: response_headers,
        body: response_body,
    })
}

#[cfg(test)]
mod tests;
