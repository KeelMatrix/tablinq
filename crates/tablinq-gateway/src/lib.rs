pub mod build_info;
mod openai;

use crate::openai::{
    ChatCompletionsRequest, PROVIDER_MODELS, model_object, openai_error, resolve_model,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use gptd::core::error_code_for_message;
use gptd::logging::JsonLogger;
use gptd::{AppConfig, CoreService};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shared_protocol::{BrowserOpenRequest, ErrorCode, EventPayload, StreamChunkKind};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const HEALTH_PATH: &str = "/health";
const READY_PATH: &str = "/ready";
const ADMIN_STATUS_PATH: &str = "/admin/status";
const ADMIN_SHUTDOWN_PATH: &str = "/admin/shutdown";
const V1_MODELS_PATH: &str = "/v1/models";
const V1_CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayStatusSnapshot {
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub state_root: String,
    pub host: String,
    pub port: u16,
    pub api_key_present: bool,
    pub max_concurrent_upstream_jobs: u32,
    pub queue_length: u32,
    pub request_timeout_ms: u64,
    pub safe_mode_behavior: String,
    pub log_path: String,
}

#[derive(Debug)]
pub struct LockGuard {
    path: PathBuf,
}

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
#[allow(dead_code)]
struct SimpleHttpResponse {
    status_code: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

struct UpstreamSlot<'a> {
    counter: &'a AtomicU32,
}

impl<'a> UpstreamSlot<'a> {
    fn try_acquire(counter: &'a AtomicU32, limit: u32) -> Option<Self> {
        let mut current = counter.load(Ordering::SeqCst);
        loop {
            if current >= limit {
                return None;
            }
            match counter.compare_exchange(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return Some(Self { counter }),
                Err(observed) => current = observed,
            }
        }
    }
}

impl Drop for UpstreamSlot<'_> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn run_gateway(config: AppConfig) -> Result<()> {
    let log_path = gateway_log_path(&config);
    let logger = Arc::new(JsonLogger::new(&log_path)?);
    let core = Arc::new(CoreService::new(config.clone())?);
    let lock = acquire_gateway_lock(&config)?;

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

    if config.warm_mode() {
        if let Err(err) = core.warm_browser_if_enabled() {
            logger.warn(
                "gateway warm browser open failed",
                json!({
                    "error": format!("{err:#}"),
                }),
            )?;
        }
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
        request_timeout_ms: config.gateway_request_timeout_ms(),
        safe_mode_behavior: config.gateway_safe_mode_behavior().to_owned(),
        log_path: log_path.display().to_string(),
    });
    write_status_snapshot(&config, &snapshot)?;

    logger.info(
        "gateway ready",
        json!({
            "request_id": "bootstrap",
            "host": snapshot.host,
            "port": snapshot.port,
            "safe_mode_behavior": snapshot.safe_mode_behavior,
            "max_concurrent_upstream_jobs": snapshot.max_concurrent_upstream_jobs,
            "queue_length": snapshot.queue_length,
        }),
    )?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let request_counter = Arc::new(AtomicU64::new(0));
    let active_upstream = Arc::new(AtomicU32::new(0));

    while !shutdown.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, addr)) => {
                let logger = Arc::clone(&logger);
                let core = Arc::clone(&core);
                let snapshot = Arc::clone(&snapshot);
                let shutdown = Arc::clone(&shutdown);
                let request_counter = Arc::clone(&request_counter);
                let active_upstream = Arc::clone(&active_upstream);
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
                        &active_upstream,
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

    remove_status_snapshot(&config)?;
    drop(lock);
    Ok(())
}

fn handle_connection(
    mut stream: TcpStream,
    remote_addr: String,
    config: &AppConfig,
    core: &CoreService,
    snapshot: &GatewayStatusSnapshot,
    shutdown: &AtomicBool,
    request_counter: &AtomicU64,
    active_upstream: &AtomicU32,
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
            logger.warn(
                "gateway request parse failed",
                json!({
                    "request_id": request_id,
                    "error": format!("{err:#}"),
                }),
            )?;
            return write_http_response(
                &mut stream,
                &request_id,
                json_response(
                    400,
                    json!({
                        "error": "bad_request",
                    }),
                ),
            );
        }
    };

    log_request(logger, &context.request_id, &context.request, remote_addr)?;

    let response = match (context.request.method.as_str(), context.request.path.as_str()) {
        ("GET", HEALTH_PATH) => json_response(
            200,
            json!({
                "service": "tablinq-gateway",
                "request_id": context.request_id,
                "status": core.health_snapshot()?.status,
                "health": core.health_snapshot()?,
                "gateway": snapshot,
            }),
        ),
        ("GET", READY_PATH) => json_response(
            200,
            json!({
                "service": "tablinq-gateway",
                "request_id": context.request_id,
                "ready": true,
                "gateway": snapshot,
            }),
        ),
        ("GET", ADMIN_STATUS_PATH) => match authorize_request(config, &context.request, false) {
            Ok(()) => json_response(
                200,
                json!({
                    "request_id": context.request_id,
                    "gateway": snapshot,
                }),
            ),
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
            Ok(()) => handle_models_request(core, snapshot, &context.request_id),
            Err(response) => response,
        },
        ("POST", V1_CHAT_COMPLETIONS_PATH) => {
            if let Err(response) = authorize_request(config, &context.request, true) {
                response
            } else if let Some(_slot) = UpstreamSlot::try_acquire(
                active_upstream,
                config.gateway_max_concurrent_upstream_jobs(),
            ) {
                if let Err(err) = handle_chat_completions_request(
                    &mut stream,
                    core,
                    &context,
                    logger,
                ) {
                    let response = map_openai_error(&context.request_id, &err);
                    log_response(logger, &context.request_id, response.status_code)?;
                    return write_http_response(&mut stream, &context.request_id, response);
                }
                return Ok(());
            } else {
                json_response(
                    429,
                    openai_error(
                        429,
                        "overloaded_error",
                        "queue_full",
                        "the gateway is at its upstream concurrency limit",
                    ),
                )
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
    snapshot: &GatewayStatusSnapshot,
    request_id: &str,
) -> HttpResponse {
    let capability_report = discover_capabilities(core);
    let models = PROVIDER_MODELS
        .iter()
        .map(|spec| model_object(spec, capability_report.as_ref()))
        .collect::<Vec<_>>();
    json_response(
        200,
        json!({
            "object": "list",
            "data": models,
            "request_id": request_id,
            "gateway": {
                "host": snapshot.host,
                "port": snapshot.port,
            }
        }),
    )
}

fn handle_chat_completions_request(
    stream: &mut TcpStream,
    core: &CoreService,
    context: &RequestContext,
    logger: &JsonLogger,
) -> Result<()> {
    let request: ChatCompletionsRequest =
        serde_json::from_slice(&context.request.body).context("decode chat completions request")?;
    openai::validate_chat_request(&request)?;
    let model = resolve_model(&request.model).context("resolve provider model")?;
    let (prompt, files) = openai::flatten_messages(&request)?;
    ensure_browser_session(core)?;

    if request.stream {
        let stream_id = format!("chatcmpl-{}", Utc::now().timestamp_millis());
        write_sse_headers(stream, &context.request_id)?;
        write_sse_event(
            stream,
            &json!({
                "id": stream_id,
                "object": "chat.completion.chunk",
                "created": Utc::now().timestamp(),
                "model": model.id,
                "request_id": context.request_id,
                "choices": [{
                    "index": 0,
                    "delta": { "role": "assistant" },
                    "finish_reason": Value::Null,
                }],
            }),
        )?;

        let result = if model.uses_research {
            core.research(
                shared_protocol::ResearchCommand {
                    query: prompt,
                    files,
                    output_dir: None,
                },
                |event| write_stream_delta(stream, &stream_id, model.id, &context.request_id, event),
            )
            .map(|result| result.report_text)
        } else {
            core.ask(
                shared_protocol::AskCommand {
                    prompt,
                    conversation_id: None,
                    mode: model.mode,
                    model_class: model.model_class,
                    effort: model.effort,
                    files,
                },
                |event| write_stream_delta(stream, &stream_id, model.id, &context.request_id, event),
            )
            .map(|result| result.output_text)
        };

        match result {
            Ok(_) => {
                write_sse_event(
                    stream,
                    &json!({
                        "id": stream_id,
                        "object": "chat.completion.chunk",
                        "created": Utc::now().timestamp(),
                        "model": model.id,
                        "request_id": context.request_id,
                        "choices": [{
                            "index": 0,
                            "delta": {},
                            "finish_reason": "stop",
                        }],
                    }),
                )?;
                write_sse_done(stream)?;
                logger.info(
                    "gateway streamed chat completion",
                    json!({
                        "request_id": context.request_id,
                        "model": model.id,
                    }),
                )?;
                Ok(())
            }
            Err(err) => {
                write_sse_event(stream, &map_openai_error_value(&context.request_id, &err))?;
                write_sse_done(stream)?;
                Ok(())
            }
        }
    } else {
        let response = if model.uses_research {
            let result = core.research(
                shared_protocol::ResearchCommand {
                    query: prompt,
                    files,
                    output_dir: None,
                },
                |_event| Ok(()),
            )?;
            build_chat_completion_response(
                &context.request_id,
                model.id,
                &result.report_text,
                "stop",
            )
        } else {
            let result = core.ask(
                shared_protocol::AskCommand {
                    prompt,
                    conversation_id: None,
                    mode: model.mode,
                    model_class: model.model_class,
                    effort: model.effort,
                    files,
                },
                |_event| Ok(()),
            )?;
            build_chat_completion_response(
                &context.request_id,
                model.id,
                &result.output_text,
                "stop",
            )
        };
        log_response(logger, &context.request_id, response.status_code)?;
        write_http_response(stream, &context.request_id, response)
    }
}

fn write_stream_delta(
    stream: &mut TcpStream,
    stream_id: &str,
    model_id: &str,
    request_id: &str,
    event: EventPayload,
) -> Result<()> {
    if let EventPayload::StreamChunk(chunk) = event {
        if chunk.kind == StreamChunkKind::Delta && !chunk.text.is_empty() {
            write_sse_event(
                stream,
                &json!({
                    "id": stream_id,
                    "object": "chat.completion.chunk",
                    "created": Utc::now().timestamp(),
                    "model": model_id,
                    "request_id": request_id,
                    "choices": [{
                        "index": 0,
                        "delta": { "content": chunk.text },
                        "finish_reason": Value::Null,
                    }],
                }),
            )?;
        }
    }
    Ok(())
}

fn build_chat_completion_response(
    request_id: &str,
    model_id: &str,
    content: &str,
    finish_reason: &str,
) -> HttpResponse {
    json_response(
        200,
        json!({
            "id": format!("chatcmpl-{}", Utc::now().timestamp_millis()),
            "object": "chat.completion",
            "created": Utc::now().timestamp(),
            "model": model_id,
            "request_id": request_id,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content,
                },
                "finish_reason": finish_reason,
            }],
            "usage": Value::Null,
            "warnings": ["usage is unavailable for the browser-backed provider"],
        }),
    )
}

fn ensure_browser_session(core: &CoreService) -> Result<()> {
    let snapshot = core.browser_profile_info()?;
    if snapshot.running {
        return Ok(());
    }
    core.browser_open(BrowserOpenRequest {
        target_url: None,
        browser_path: None,
        headless: Some(true),
    })?;
    Ok(())
}

fn discover_capabilities(core: &CoreService) -> Option<shared_protocol::CapabilityReport> {
    if let Ok(snapshot) = core.browser_profile_info() {
        if !snapshot.running {
            let _ = core.browser_open(BrowserOpenRequest {
                target_url: None,
                browser_path: None,
                headless: Some(true),
            });
        }
    }
    core.capabilities().ok().and_then(|snapshot| snapshot.report)
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

fn map_openai_error(request_id: &str, err: &anyhow::Error) -> HttpResponse {
    let (status, error_type, code, message) = map_openai_error_parts(err);
    json_response(
        status,
        openai_error(status, error_type, code, &format!("{request_id}: {message}")),
    )
}

fn map_openai_error_value(request_id: &str, err: &anyhow::Error) -> Value {
    let (status, error_type, code, message) = map_openai_error_parts(err);
    let mut value = openai_error(status, error_type, code, &message);
    value["request_id"] = Value::String(request_id.to_owned());
    value
}

fn map_openai_error_parts(err: &anyhow::Error) -> (u16, &'static str, &'static str, String) {
    let message = format!("{err:#}");
    match error_code_for_message(&message) {
        ErrorCode::CapabilityMissing => (
            400,
            "invalid_request_error",
            "capability_missing",
            message,
        ),
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
            } else {
                (500, "server_error", "transport_error", message)
            }
        }
    }
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

pub fn start_gateway_process(state_root: Option<&Path>) -> Result<Value> {
    start_gateway_process_with_binary(state_root, None)
}

fn start_gateway_process_with_binary(
    state_root: Option<&Path>,
    binary_override: Option<&Path>,
) -> Result<Value> {
    let config = AppConfig::load_or_create(state_root)?;
    if let Some(snapshot) = load_status_snapshot(&config)? {
        if gateway_is_responsive(&snapshot, config.gateway_request_timeout_ms()) {
            return Ok(json!({
                "status": "already_running",
                "host": snapshot.host,
                "port": snapshot.port,
                "pid": snapshot.pid,
            }));
        }
        remove_status_snapshot(&config)?;
        let _ = fs::remove_file(gateway_lock_path(&config));
    }

    let gateway_bin = binary_override
        .map(Path::to_path_buf)
        .unwrap_or_else(resolve_gateway_binary);
    let mut command = Command::new(&gateway_bin);
    command
        .arg("serve")
        .arg("--state-root")
        .arg(&config.paths.state_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("spawn gateway binary {}", gateway_bin.display()))?;

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if let Some(snapshot) = load_status_snapshot(&config)? {
            if gateway_is_ready(&snapshot, config.gateway_request_timeout_ms()) {
                return Ok(json!({
                    "status": "started",
                    "host": snapshot.host,
                    "port": snapshot.port,
                    "pid": snapshot.pid,
                }));
            }
        }

        if let Some(status) = child.try_wait().context("poll gateway startup")? {
            bail!("gateway exited during startup with status {status}");
        }

        if Instant::now() >= deadline {
            bail!("gateway did not become ready within 5 seconds");
        }

        thread::sleep(Duration::from_millis(100));
    }
}

pub fn gateway_status(state_root: Option<&Path>) -> Result<Value> {
    let config = AppConfig::load_or_create(state_root)?;
    let snapshot = load_status_snapshot(&config)?;
    if let Some(snapshot) = snapshot {
        let healthy = gateway_is_responsive(&snapshot, config.gateway_request_timeout_ms());
        let ready = gateway_is_ready(&snapshot, config.gateway_request_timeout_ms());
        Ok(json!({
            "running": healthy,
            "ready": ready,
            "host": snapshot.host,
            "port": snapshot.port,
            "pid": snapshot.pid,
            "max_concurrent_upstream_jobs": snapshot.max_concurrent_upstream_jobs,
            "queue_length": snapshot.queue_length,
            "request_timeout_ms": snapshot.request_timeout_ms,
            "safe_mode_behavior": snapshot.safe_mode_behavior,
        }))
    } else {
        Ok(json!({
            "running": false,
            "ready": false,
        }))
    }
}

pub fn stop_gateway_process(state_root: Option<&Path>) -> Result<Value> {
    let config = AppConfig::load_or_create(state_root)?;
    let snapshot = match load_status_snapshot(&config)? {
        Some(snapshot) => snapshot,
        None => {
            return Ok(json!({
                "status": "not_running",
            }));
        }
    };

    if !gateway_is_responsive(&snapshot, config.gateway_request_timeout_ms()) {
        remove_status_snapshot(&config)?;
        let _ = fs::remove_file(gateway_lock_path(&config));
        return Ok(json!({
            "status": "not_running",
        }));
    }

    let auth_header = format!(
        "Bearer {}",
        config
            .gateway_api_key()
            .context("gateway API key is required to stop the gateway")?
    );
    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        ADMIN_SHUTDOWN_PATH,
        &[("authorization", auth_header.as_str())],
        &[],
        config.gateway_request_timeout_ms(),
    )?;
    if response.status_code != 200 {
        bail!(
            "gateway shutdown request failed with status {}",
            response.status_code
        );
    }

    let deadline = Instant::now() + STARTUP_TIMEOUT;
    while Instant::now() < deadline {
        if !gateway_is_responsive(&snapshot, config.gateway_request_timeout_ms()) {
            remove_status_snapshot(&config)?;
            let _ = fs::remove_file(gateway_lock_path(&config));
            return Ok(json!({
                "status": "stopped",
                "host": snapshot.host,
                "port": snapshot.port,
            }));
        }
        thread::sleep(Duration::from_millis(100));
    }

    bail!("gateway did not stop within 5 seconds")
}

pub fn load_status_snapshot(config: &AppConfig) -> Result<Option<GatewayStatusSnapshot>> {
    let path = gateway_status_path(config);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let snapshot = serde_json::from_str(&raw).context("decode gateway status snapshot")?;
    Ok(Some(snapshot))
}

fn write_status_snapshot(config: &AppConfig, snapshot: &GatewayStatusSnapshot) -> Result<()> {
    let path = gateway_status_path(config);
    let raw = serde_json::to_vec_pretty(snapshot).context("encode gateway status snapshot")?;
    fs::write(&path, raw).with_context(|| format!("write {}", path.display()))
}

fn remove_status_snapshot(config: &AppConfig) -> Result<()> {
    let path = gateway_status_path(config);
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(())
}

fn gateway_status_path(config: &AppConfig) -> PathBuf {
    config.paths.state_root.join("gateway-status.json")
}

fn gateway_lock_path(config: &AppConfig) -> PathBuf {
    config.paths.state_root.join("gateway.lock")
}

fn gateway_log_path(config: &AppConfig) -> PathBuf {
    config.paths.logs_dir.join("gateway.log")
}

fn acquire_gateway_lock(config: &AppConfig) -> Result<LockGuard> {
    let path = gateway_lock_path(config);
    if path.exists() {
        if let Some(snapshot) = load_status_snapshot(config)? {
            if gateway_is_responsive(&snapshot, config.gateway_request_timeout_ms()) {
                bail!("gateway already running");
            }
        }
        fs::remove_file(&path).with_context(|| format!("remove stale {}", path.display()))?;
    }

    let metadata = json!({
        "pid": std::process::id(),
        "created_at": Utc::now().to_rfc3339(),
    });
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("create {}", path.display()))?;
    file.write_all(
        serde_json::to_string_pretty(&metadata)
            .context("encode gateway lock metadata")?
            .as_bytes(),
    )
    .with_context(|| format!("write {}", path.display()))?;

    Ok(LockGuard { path })
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn resolve_gateway_binary() -> PathBuf {
    if let Ok(value) = std::env::var("TABLINQ_GATEWAY_BIN") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    let current = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("gpt"));
    let mut sibling = current.clone();
    sibling.set_file_name(if cfg!(windows) {
        "tablinq-gateway.exe"
    } else {
        "tablinq-gateway"
    });
    if sibling.exists() {
        sibling
    } else {
        PathBuf::from(if cfg!(windows) {
            "tablinq-gateway.exe"
        } else {
            "tablinq-gateway"
        })
    }
}

fn gateway_is_responsive(snapshot: &GatewayStatusSnapshot, timeout_ms: u64) -> bool {
    http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        HEALTH_PATH,
        &[],
        &[],
        timeout_ms,
    )
    .map(|response| response.status_code == 200)
    .unwrap_or(false)
}

fn gateway_is_ready(snapshot: &GatewayStatusSnapshot, timeout_ms: u64) -> bool {
    http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        READY_PATH,
        &[],
        &[],
        timeout_ms,
    )
    .map(|response| response.status_code == 200)
    .unwrap_or(false)
}

fn log_request(
    logger: &JsonLogger,
    request_id: &str,
    request: &HttpRequest,
    remote_addr: String,
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
    let reason_phrase = match status_code {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Internal Server Error",
    };
    let body = serde_json::to_vec(&body).expect("serialize JSON response");
    HttpResponse {
        status_code,
        reason_phrase,
        headers: vec![
            ("content-type".to_owned(), "application/json".to_owned()),
            ("connection".to_owned(), "close".to_owned()),
        ],
        body,
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
        headers: response_headers,
        body: response_body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::path::Path;
    use std::process::{Child, Stdio};
    use tempfile::{TempDir, tempdir};

    fn spawn_fake_chatgpt() -> (Child, String) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("allocate tcp port");
        let port = listener.local_addr().expect("resolve local address").port();
        drop(listener);

        let server_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("packages")
            .join("fake-chatgpt")
            .join("server.mjs");
        let child = Command::new("node")
            .arg(server_path)
            .env("PORT", port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn fake-chatgpt");

        let url = format!("http://127.0.0.1:{port}");
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return (child, url);
            }
            thread::sleep(Duration::from_millis(100));
        }

        panic!("fake-chatgpt did not become ready on port {port}");
    }

    fn spawn_gateway(config: AppConfig) -> (thread::JoinHandle<Result<()>>, GatewayStatusSnapshot) {
        let state_root = config.paths.state_root.clone();
        let handle = thread::spawn(move || run_gateway(config));
        let config = AppConfig::load_or_create(Some(&state_root)).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(snapshot) = load_status_snapshot(&config).unwrap() {
                return (handle, snapshot);
            }
            assert!(
                Instant::now() < deadline,
                "gateway did not write status snapshot"
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn test_config() -> (TempDir, AppConfig) {
        let dir = tempdir().unwrap();
        let mut config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.port = Some(0);
            gateway.request_timeout_ms = Some(20_000);
        }
        (dir, config)
    }

    fn auth_header(config: &AppConfig) -> String {
        format!(
            "Bearer {}",
            config.gateway_api_key().expect("gateway api key")
        )
    }

    #[test]
    fn gateway_health_and_readiness_endpoints_respond() {
        let (_dir, config) = test_config();
        let state_root = config.paths.state_root.clone();
        let (handle, snapshot) = spawn_gateway(config);

        let health = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            HEALTH_PATH,
            &[],
            &[],
            3_000,
        )
        .unwrap();
        assert_eq!(health.status_code, 200);

        let ready = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            READY_PATH,
            &[],
            &[],
            3_000,
        )
        .unwrap();
        assert_eq!(ready.status_code, 200);

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn gateway_rejects_unauthorized_admin_requests() {
        let (_dir, config) = test_config();
        let state_root = config.paths.state_root.clone();
        let (handle, snapshot) = spawn_gateway(config);

        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            ADMIN_SHUTDOWN_PATH,
            &[],
            &[],
            3_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 401);

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn gateway_propagates_request_ids_in_headers_and_logs() {
        let (_dir, config) = test_config();
        let state_root = config.paths.state_root.clone();
        let log_path = gateway_log_path(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            HEALTH_PATH,
            &[("x-request-id", "req-test-123")],
            &[],
            3_000,
        )
        .unwrap();
        assert_eq!(
            response.headers.get("x-request-id").map(String::as_str),
            Some("req-test-123")
        );

        let log = fs::read_to_string(log_path).unwrap();
        assert!(log.contains("req-test-123"));

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn gateway_status_and_stop_helpers_work_against_running_service() {
        let (_dir, config) = test_config();
        let state_root = config.paths.state_root.clone();
        let (handle, _snapshot) = spawn_gateway(config);

        let status = gateway_status(Some(&state_root)).unwrap();
        assert_eq!(status["running"], true);
        assert_eq!(status["ready"], true);

        let stop = stop_gateway_process(Some(&state_root)).unwrap();
        assert_eq!(stop["status"], "stopped");
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn models_endpoint_returns_openai_model_catalog() {
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(fake_url);
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            V1_MODELS_PATH,
            &[("authorization", auth.as_str())],
            &[],
            10_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 200);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["object"], "list");
        assert!(body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "tablinq-instant"));
        assert!(body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == "tablinq-search"));

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_non_streaming_returns_openai_shape() {
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(fake_url);
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [
                { "role": "system", "content": "Answer briefly." },
                { "role": "user", "content": "hello from gateway" }
            ]
        }))
        .unwrap();
        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
            ],
            &body,
            20_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 200);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["object"], "chat.completion");
        assert_eq!(body["model"], "tablinq-instant");
        assert_eq!(body["choices"][0]["message"]["role"], "assistant");
        assert!(body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap()
            .contains("Fake response for"));
        assert_eq!(body["choices"][0]["finish_reason"], "stop");

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_streaming_emits_sse_chunks_and_done() {
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(fake_url);
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let body = serde_json::to_vec(&json!({
            "model": "tablinq-search",
            "stream": true,
            "messages": [
                { "role": "user", "content": "latest roadmap" }
            ]
        }))
        .unwrap();
        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
            ],
            &body,
            20_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 200);
        assert_eq!(
            response.headers.get("content-type").map(String::as_str),
            Some("text/event-stream")
        );
        let text = String::from_utf8(response.body).unwrap();
        assert!(text.contains("chat.completion.chunk"));
        assert!(text.contains("[DONE]"));

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_maps_capability_errors_to_openai_shape() {
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?missing=search"));
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let body = serde_json::to_vec(&json!({
            "model": "tablinq-search",
            "messages": [
                { "role": "user", "content": "search for release notes" }
            ]
        }))
        .unwrap();
        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
            ],
            &body,
            20_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 400);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["error"]["code"], "capability_missing");

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_reject_when_gateway_is_busy() {
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=timeout"));
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let first_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [
                { "role": "user", "content": "this will stall" }
            ]
        }))
        .unwrap();
        let host = snapshot.host.clone();
        let port = snapshot.port;
        let auth_for_thread = auth.clone();
        let first = thread::spawn(move || {
            http_request(
                &host,
                port,
                "POST",
                V1_CHAT_COMPLETIONS_PATH,
                &[
                    ("authorization", auth_for_thread.as_str()),
                    ("content-type", "application/json"),
                ],
                &first_body,
                20_000,
            )
        });

        thread::sleep(Duration::from_millis(500));

        let second_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [
                { "role": "user", "content": "hello" }
            ]
        }))
        .unwrap();
        let second = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
            ],
            &second_body,
            20_000,
        )
        .unwrap();
        assert_eq!(second.status_code, 429);
        let body: Value = serde_json::from_slice(&second.body).unwrap();
        assert_eq!(body["error"]["code"], "queue_full");

        let first = first.join().unwrap().unwrap();
        assert_eq!(first.status_code, 504);

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }
}
