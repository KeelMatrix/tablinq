pub mod build_info;
mod openai;

use crate::openai::{
    ChatCompletionsRequest, PROVIDER_MODELS, model_object, openai_error, resolve_model,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use gptd::core::{PreparedAskJob, PreparedResearchJob, error_code_for_message};
use gptd::logging::JsonLogger;
use gptd::{AppConfig, CoreService};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shared_protocol::{
    BrowserOpenRequest, BrowserStatusSnapshot, CapabilityReport, ErrorCode, EventPayload,
    StreamChunkKind,
};
use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const HEALTH_PATH: &str = "/health";
const READY_PATH: &str = "/ready";
const ADMIN_STATUS_PATH: &str = "/admin/status";
const ADMIN_SHUTDOWN_PATH: &str = "/admin/shutdown";
const V1_MODELS_PATH: &str = "/v1/models";
const V1_CAPABILITIES_PATH: &str = "/v1/capabilities";
const V1_CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
const V1_REQUESTS_PREFIX: &str = "/v1/requests/";

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
    pub queue_timeout_ms: u64,
    pub per_client_limit: u32,
    pub request_timeout_ms: u64,
    pub ask_timeout_ms: u64,
    pub research_timeout_ms: u64,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CapabilityCacheEntry {
    cache_key: String,
    profile_dir: String,
    workspace_key: String,
    cached_at: DateTime<Utc>,
    report: CapabilityReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CapabilityCacheState {
    entries: BTreeMap<String, CapabilityCacheEntry>,
}

#[derive(Debug, Clone)]
struct CapabilityCatalog {
    source: &'static str,
    cache_key: String,
    profile_dir: String,
    workspace_key: String,
    report: Option<CapabilityReport>,
}

#[derive(Debug)]
struct CapabilityCatalogCache {
    path: PathBuf,
    inner: Mutex<CapabilityCacheState>,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct SchedulerSnapshot {
    running: u32,
    queued: u32,
    max_concurrent_upstream_jobs: u32,
    queue_length: u32,
    queue_timeout_ms: u64,
    per_client_limit: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestState {
    Queued,
    Running,
}

#[derive(Debug, Clone)]
struct TrackedRequest {
    job_id: String,
    client_id: String,
    state: RequestState,
    cancelled: bool,
}

#[derive(Debug, Default)]
struct SchedulerState {
    running: u32,
    queue: VecDeque<String>,
    requests: BTreeMap<String, TrackedRequest>,
}

#[derive(Debug)]
struct GatewayScheduler {
    max_concurrent_upstream_jobs: u32,
    queue_length: u32,
    queue_timeout: Duration,
    per_client_limit: u32,
    inner: Mutex<SchedulerState>,
    changed: Condvar,
}

#[derive(Debug)]
struct SchedulerLease<'a> {
    scheduler: &'a GatewayScheduler,
    request_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CancelDisposition {
    Queued,
    Running,
}

impl GatewayScheduler {
    fn new(config: &AppConfig) -> Self {
        Self {
            max_concurrent_upstream_jobs: config.gateway_max_concurrent_upstream_jobs(),
            queue_length: config.gateway_queue_length(),
            queue_timeout: Duration::from_millis(config.gateway_queue_timeout_ms()),
            per_client_limit: config.gateway_per_client_limit(),
            inner: Mutex::new(SchedulerState::default()),
            changed: Condvar::new(),
        }
    }

    fn acquire<'a>(
        &'a self,
        request_id: &str,
        job_id: &str,
        client_id: &str,
    ) -> Result<SchedulerLease<'a>> {
        let mut state = lock_scheduler(&self.inner)?;
        if client_request_count(&state, client_id) >= self.per_client_limit {
            bail!("per-client queue limit reached");
        }
        if state.running < self.max_concurrent_upstream_jobs && state.queue.is_empty() {
            state.running += 1;
            state.requests.insert(
                request_id.to_owned(),
                TrackedRequest {
                    job_id: job_id.to_owned(),
                    client_id: client_id.to_owned(),
                    state: RequestState::Running,
                    cancelled: false,
                },
            );
            return Ok(SchedulerLease {
                scheduler: self,
                request_id: request_id.to_owned(),
            });
        }

        if state.queue.len() as u32 >= self.queue_length {
            bail!("queue full");
        }
        state.queue.push_back(request_id.to_owned());
        state.requests.insert(
            request_id.to_owned(),
            TrackedRequest {
                job_id: job_id.to_owned(),
                client_id: client_id.to_owned(),
                state: RequestState::Queued,
                cancelled: false,
            },
        );

        let deadline = Instant::now() + self.queue_timeout;
        loop {
            if let Some(entry) = state.requests.get(request_id) {
                if entry.cancelled {
                    remove_queued_request(&mut state, request_id);
                    self.changed.notify_all();
                    bail!("request cancelled while queued");
                }
            }

            let ready = state.running < self.max_concurrent_upstream_jobs
                && state.queue.front().map(String::as_str) == Some(request_id);
            if ready {
                state.queue.pop_front();
                state.running += 1;
                if let Some(entry) = state.requests.get_mut(request_id) {
                    entry.state = RequestState::Running;
                }
                return Ok(SchedulerLease {
                    scheduler: self,
                    request_id: request_id.to_owned(),
                });
            }

            let now = Instant::now();
            if now >= deadline {
                remove_queued_request(&mut state, request_id);
                self.changed.notify_all();
                bail!("queue timeout exceeded");
            }

            let timeout = deadline.saturating_duration_since(now);
            let (next_state, wait_result) = self
                .changed
                .wait_timeout(state, timeout)
                .map_err(|_| anyhow::anyhow!("gateway scheduler lock poisoned"))?;
            state = next_state;
            if wait_result.timed_out() {
                remove_queued_request(&mut state, request_id);
                self.changed.notify_all();
                bail!("queue timeout exceeded");
            }
        }
    }

    fn cancel(&self, request_id: &str) -> Result<Option<(String, CancelDisposition)>> {
        let mut state = lock_scheduler(&self.inner)?;
        let Some(entry) = state.requests.get_mut(request_id) else {
            return Ok(None);
        };
        entry.cancelled = true;
        let disposition = match entry.state {
            RequestState::Queued => CancelDisposition::Queued,
            RequestState::Running => CancelDisposition::Running,
        };
        let job_id = entry.job_id.clone();
        self.changed.notify_all();
        Ok(Some((job_id, disposition)))
    }

    fn snapshot(&self) -> Result<SchedulerSnapshot> {
        let state = lock_scheduler(&self.inner)?;
        Ok(SchedulerSnapshot {
            running: state.running,
            queued: state.queue.len() as u32,
            max_concurrent_upstream_jobs: self.max_concurrent_upstream_jobs,
            queue_length: self.queue_length,
            queue_timeout_ms: self.queue_timeout.as_millis() as u64,
            per_client_limit: self.per_client_limit,
        })
    }
}

impl Drop for SchedulerLease<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.scheduler.inner.lock() {
            if state.requests.remove(&self.request_id).is_some() && state.running > 0 {
                state.running -= 1;
            }
            self.scheduler.changed.notify_all();
        }
    }
}

fn lock_scheduler<'a>(
    mutex: &'a Mutex<SchedulerState>,
) -> Result<std::sync::MutexGuard<'a, SchedulerState>> {
    mutex
        .lock()
        .map_err(|_| anyhow::anyhow!("gateway scheduler lock poisoned"))
}

fn remove_queued_request(state: &mut SchedulerState, request_id: &str) {
    state.requests.remove(request_id);
    if let Some(index) = state.queue.iter().position(|value| value == request_id) {
        state.queue.remove(index);
    }
}

fn client_request_count(state: &SchedulerState, client_id: &str) -> u32 {
    state
        .requests
        .values()
        .filter(|entry| entry.client_id == client_id)
        .count() as u32
}

impl CapabilityCatalogCache {
    fn load(config: &AppConfig) -> Result<Self> {
        let path = capability_cache_path(config);
        let state = if path.exists() {
            let raw =
                fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
            serde_json::from_str(&raw).context("decode capability cache")?
        } else {
            CapabilityCacheState::default()
        };
        Ok(Self {
            path,
            inner: Mutex::new(state),
        })
    }

    fn resolve(&self, core: &CoreService) -> Result<CapabilityCatalog> {
        let live = discover_capabilities(core);
        let profile = core.browser_profile_info().ok();
        let (cache_key, profile_dir, workspace_key) = catalog_identity(profile.as_ref(), core);

        if let Some(report) = live {
            self.remember(&cache_key, &profile_dir, &workspace_key, &report)?;
            return Ok(CapabilityCatalog {
                source: "live",
                cache_key,
                profile_dir,
                workspace_key,
                report: Some(report),
            });
        }

        if let Some(entry) = self.cached(&cache_key)? {
            return Ok(CapabilityCatalog {
                source: "cached",
                cache_key,
                profile_dir,
                workspace_key,
                report: Some(entry.report),
            });
        }

        Ok(CapabilityCatalog {
            source: "unknown",
            cache_key,
            profile_dir,
            workspace_key,
            report: None,
        })
    }

    fn cached(&self, cache_key: &str) -> Result<Option<CapabilityCacheEntry>> {
        let state = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("capability cache lock poisoned"))?;
        Ok(state.entries.get(cache_key).cloned())
    }

    fn remember(
        &self,
        cache_key: &str,
        profile_dir: &str,
        workspace_key: &str,
        report: &CapabilityReport,
    ) -> Result<()> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("capability cache lock poisoned"))?;
        state.entries.insert(
            cache_key.to_owned(),
            CapabilityCacheEntry {
                cache_key: cache_key.to_owned(),
                profile_dir: profile_dir.to_owned(),
                workspace_key: workspace_key.to_owned(),
                cached_at: Utc::now(),
                report: report.clone(),
            },
        );
        let raw = serde_json::to_vec_pretty(&*state).context("encode capability cache")?;
        fs::write(&self.path, raw).with_context(|| format!("write {}", self.path.display()))
    }
}

fn capability_cache_path(config: &AppConfig) -> PathBuf {
    config
        .paths
        .state_root
        .join("gateway-capability-cache.json")
}

fn catalog_identity(
    snapshot: Option<&BrowserStatusSnapshot>,
    core: &CoreService,
) -> (String, String, String) {
    let profile_dir = snapshot
        .map(|value| value.profile_dir.clone())
        .unwrap_or_else(|| {
            core.config()
                .paths
                .browser_profile_dir
                .display()
                .to_string()
        });
    let workspace_key = snapshot
        .and_then(|value| value.current_url.clone().or(Some(value.target_url.clone())))
        .unwrap_or_else(|| core.config().browser_target_url().to_owned());
    (
        format!("{profile_dir}::{workspace_key}"),
        profile_dir,
        workspace_key,
    )
}

pub fn run_gateway(config: AppConfig) -> Result<()> {
    let log_path = gateway_log_path(&config);
    let logger = Arc::new(JsonLogger::new(&log_path)?);
    let core = Arc::new(CoreService::new(config.clone())?);
    let capability_cache = Arc::new(CapabilityCatalogCache::load(&config)?);
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
        queue_timeout_ms: config.gateway_queue_timeout_ms(),
        per_client_limit: config.gateway_per_client_limit(),
        request_timeout_ms: config.gateway_request_timeout_ms(),
        ask_timeout_ms: config.ask_timeout_ms(),
        research_timeout_ms: config.research_timeout_ms(),
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
    scheduler: &GatewayScheduler,
    capability_cache: &CapabilityCatalogCache,
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

    log_request(logger, &context.request_id, &context.request, &remote_addr)?;
    let scheduler_status = scheduler.snapshot()?;

    let response = match (
        context.request.method.as_str(),
        context.request.path.as_str(),
    ) {
        ("GET", HEALTH_PATH) => json_response(
            200,
            json!({
                "service": "tablinq-gateway",
                "request_id": context.request_id,
                "status": core.health_snapshot()?.status,
                "health": core.health_snapshot()?,
                "gateway": snapshot,
                "scheduler": scheduler_status,
            }),
        ),
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
            Ok(()) => json_response(
                200,
                json!({
                    "request_id": context.request_id,
                    "gateway": snapshot,
                    "scheduler": scheduler_status,
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
            Ok(()) => handle_models_request(core, capability_cache, snapshot, &context.request_id),
            Err(response) => response,
        },
        ("GET", V1_CAPABILITIES_PATH) => match authorize_request(config, &context.request, true) {
            Ok(()) => handle_capabilities_request(core, capability_cache, &context.request_id)?,
            Err(response) => response,
        },
        ("POST", _) if context.request.path.starts_with(V1_REQUESTS_PREFIX) => {
            if let Err(response) = authorize_request(config, &context.request, false) {
                response
            } else {
                handle_cancel_request(core, scheduler, &context)?
            }
        }
        ("POST", V1_CHAT_COMPLETIONS_PATH) => {
            if let Err(response) = authorize_request(config, &context.request, true) {
                response
            } else {
                let prepared = match prepare_chat_completions_request(core, &context) {
                    Ok(prepared) => prepared,
                    Err(err) => {
                        let response = map_openai_error(&context.request_id, &err);
                        log_response(logger, &context.request_id, response.status_code)?;
                        return write_http_response(&mut stream, &context.request_id, response);
                    }
                };
                let client_id = client_id_for_request(&context.request, &remote_addr);
                let _lease =
                    match scheduler.acquire(&context.request_id, prepared.job_id(), &client_id) {
                        Ok(lease) => lease,
                        Err(err) => {
                            let _ = core.fail_job(prepared.job_id(), &format!("{err:#}"));
                            let response = queue_error_response(
                                &context.request_id,
                                &scheduler.snapshot().unwrap_or(scheduler_status),
                                &err,
                            );
                            log_response(logger, &context.request_id, response.status_code)?;
                            return write_http_response(&mut stream, &context.request_id, response);
                        }
                    };
                if core
                    .job_cancel_requested(prepared.job_id())
                    .unwrap_or(false)
                {
                    let _ = core.fail_job(prepared.job_id(), "request cancelled before execution");
                    let response = cancellation_response(
                        &context.request_id,
                        prepared.job_id(),
                        "request cancelled before execution",
                    );
                    log_response(logger, &context.request_id, response.status_code)?;
                    return write_http_response(&mut stream, &context.request_id, response);
                }
                if let Err(err) =
                    execute_chat_completions_request(&mut stream, core, &context, logger, &prepared)
                {
                    let response = map_openai_error(&context.request_id, &err);
                    log_response(logger, &context.request_id, response.status_code)?;
                    return write_http_response(&mut stream, &context.request_id, response);
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
        }),
    ))
}

enum PreparedChatJob {
    Ask(PreparedAskJob),
    Research(PreparedResearchJob),
}

struct PreparedChatRequest {
    request: ChatCompletionsRequest,
    model_id: &'static str,
    job: PreparedChatJob,
}

impl PreparedChatRequest {
    fn job_id(&self) -> &str {
        match &self.job {
            PreparedChatJob::Ask(job) => &job.job_id,
            PreparedChatJob::Research(job) => &job.job_id,
        }
    }
}

fn prepare_chat_completions_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<PreparedChatRequest> {
    let request: ChatCompletionsRequest =
        serde_json::from_slice(&context.request.body).context("decode chat completions request")?;
    openai::validate_chat_request(&request)?;
    let model = resolve_model(&request.model).context("resolve provider model")?;
    let (prompt, files) = openai::flatten_messages(&request)?;

    let job = if model.uses_research {
        PreparedChatJob::Research(core.prepare_research(shared_protocol::ResearchCommand {
            query: prompt,
            files,
            output_dir: None,
        })?)
    } else {
        PreparedChatJob::Ask(core.prepare_ask(shared_protocol::AskCommand {
            prompt,
            conversation_id: None,
            mode: model.mode,
            model_class: model.model_class,
            effort: model.effort,
            files,
        })?)
    };

    Ok(PreparedChatRequest {
        request,
        model_id: model.id,
        job,
    })
}

fn execute_chat_completions_request(
    stream: &mut TcpStream,
    core: &CoreService,
    context: &RequestContext,
    logger: &JsonLogger,
    prepared: &PreparedChatRequest,
) -> Result<()> {
    ensure_browser_session(core)?;

    if prepared.request.stream {
        let stream_id = format!("chatcmpl-{}", Utc::now().timestamp_millis());
        write_sse_headers(stream, &context.request_id)?;
        write_sse_event(
            stream,
            &json!({
                "id": stream_id,
                "object": "chat.completion.chunk",
                "created": Utc::now().timestamp(),
                "model": prepared.model_id,
                "request_id": context.request_id,
                "tablinq_job_id": prepared.job_id(),
                "choices": [{
                    "index": 0,
                    "delta": { "role": "assistant" },
                    "finish_reason": Value::Null,
                }],
            }),
        )?;

        let result = match &prepared.job {
            PreparedChatJob::Research(job) => core
                .execute_prepared_research(job, |event| {
                    write_stream_delta(
                        stream,
                        &stream_id,
                        prepared.model_id,
                        &context.request_id,
                        event,
                    )
                })
                .map(|result| result.report_text),
            PreparedChatJob::Ask(job) => core
                .execute_prepared_ask(job, |event| {
                    write_stream_delta(
                        stream,
                        &stream_id,
                        prepared.model_id,
                        &context.request_id,
                        event,
                    )
                })
                .map(|result| result.output_text),
        };

        match result {
            Ok(_) => {
                write_sse_event(
                    stream,
                    &json!({
                        "id": stream_id,
                        "object": "chat.completion.chunk",
                        "created": Utc::now().timestamp(),
                        "model": prepared.model_id,
                        "request_id": context.request_id,
                        "tablinq_job_id": prepared.job_id(),
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
                        "model": prepared.model_id,
                        "job_id": prepared.job_id(),
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
        let response = match &prepared.job {
            PreparedChatJob::Research(job) => {
                let result = core.execute_prepared_research(job, |_event| Ok(()))?;
                build_chat_completion_response(
                    &context.request_id,
                    prepared.job_id(),
                    prepared.model_id,
                    &result.report_text,
                    "stop",
                )
            }
            PreparedChatJob::Ask(job) => {
                let result = core.execute_prepared_ask(job, |_event| Ok(()))?;
                build_chat_completion_response(
                    &context.request_id,
                    prepared.job_id(),
                    prepared.model_id,
                    &result.output_text,
                    "stop",
                )
            }
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
    job_id: &str,
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
            "tablinq_job_id": job_id,
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
            "state": match disposition {
                CancelDisposition::Queued => "queued",
                CancelDisposition::Running => "running",
            },
        }),
    ))
}

fn cancel_request_id_from_path(path: &str) -> Option<&str> {
    let suffix = path.strip_prefix(V1_REQUESTS_PREFIX)?;
    suffix.strip_suffix("/cancel")
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
    core.capabilities()
        .ok()
        .and_then(|snapshot| snapshot.report)
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
        openai_error(
            status,
            error_type,
            code,
            &format!("{request_id}: {message}"),
        ),
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
    if message.contains("cancelled") {
        return (409, "invalid_request_error", "cancelled", message);
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
            "queue_timeout_ms": snapshot.queue_timeout_ms,
            "per_client_limit": snapshot.per_client_limit,
            "request_timeout_ms": snapshot.request_timeout_ms,
            "ask_timeout_ms": snapshot.ask_timeout_ms,
            "research_timeout_ms": snapshot.research_timeout_ms,
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
    use std::sync::{Arc, Mutex, OnceLock};
    use tempfile::{TempDir, tempdir};

    fn integration_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

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
            gateway.queue_timeout_ms = Some(20_000);
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
        let _guard = integration_lock();
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
        let _guard = integration_lock();
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
        let _guard = integration_lock();
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
        let _guard = integration_lock();
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
        let _guard = integration_lock();
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
        assert_eq!(body["tablinq_capability_source"], "live");
        assert!(
            body["data"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["id"] == "tablinq-instant")
        );
        assert!(
            body["data"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["id"] == "tablinq-search")
        );

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn capabilities_endpoint_reports_alias_remaps_for_stable_models() {
        let _guard = integration_lock();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?variant=aria"));
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            V1_CAPABILITIES_PATH,
            &[("authorization", auth.as_str())],
            &[],
            10_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 200);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["object"], "tablinq.capability_catalog");
        assert_eq!(body["source"], "live");
        assert!(body["models"].as_array().unwrap().iter().any(|item| {
            item["id"] == "tablinq-thinking-standard"
                && item["tablinq_aliases"]["model_classes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|alias| alias.as_str().is_some())
        }));

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn capabilities_endpoint_marks_missing_features_unavailable() {
        let _guard = integration_lock();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?missing=search"));
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            V1_CAPABILITIES_PATH,
            &[("authorization", auth.as_str())],
            &[],
            10_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 200);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        let search = body["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "tablinq-search")
            .unwrap();
        assert_eq!(search["tablinq_availability"], "unavailable");
        assert!(!search["tablinq_warning"].as_array().unwrap().is_empty());

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_non_streaming_returns_openai_shape() {
        let _guard = integration_lock();
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
        assert!(
            body["choices"][0]["message"]["content"]
                .as_str()
                .unwrap()
                .contains("Fake response for")
        );
        assert_eq!(body["choices"][0]["finish_reason"], "stop");

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_streaming_emits_sse_chunks_and_done() {
        let _guard = integration_lock();
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
        let _guard = integration_lock();
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
    fn chat_completions_run_queued_requests_in_fifo_order() {
        let _guard = integration_lock();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.research_timeout_ms = Some(10_000);
            gateway.queue_timeout_ms = Some(10_000);
        }
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let first_body = serde_json::to_vec(&json!({
            "model": "tablinq-research",
            "messages": [{ "role": "user", "content": "hold the slot long enough to queue" }]
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
                    ("x-request-id", "req-first"),
                ],
                &first_body,
                20_000,
            )
            .unwrap()
        });

        thread::sleep(Duration::from_millis(200));

        let completions = Arc::new(Mutex::new(Vec::new()));
        let second_body = serde_json::to_vec(&json!({
            "model": "tablinq-research",
            "messages": [
                { "role": "user", "content": "hello from second" }
            ]
        }))
        .unwrap();
        let second_host = snapshot.host.clone();
        let second_auth = auth.clone();
        let second_completions = Arc::clone(&completions);
        let second = thread::spawn(move || {
            let response = http_request(
                &second_host,
                port,
                "POST",
                V1_CHAT_COMPLETIONS_PATH,
                &[
                    ("authorization", second_auth.as_str()),
                    ("content-type", "application/json"),
                    ("x-request-id", "req-second"),
                ],
                &second_body,
                20_000,
            )
            .unwrap();
            second_completions.lock().unwrap().push("second".to_owned());
            response
        });

        thread::sleep(Duration::from_millis(100));

        let third_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [
                { "role": "user", "content": "hello from third" }
            ]
        }))
        .unwrap();
        let third_host = snapshot.host.clone();
        let third_auth = auth.clone();
        let third_completions = Arc::clone(&completions);
        let third = thread::spawn(move || {
            let response = http_request(
                &third_host,
                port,
                "POST",
                V1_CHAT_COMPLETIONS_PATH,
                &[
                    ("authorization", third_auth.as_str()),
                    ("content-type", "application/json"),
                    ("x-request-id", "req-third"),
                ],
                &third_body,
                20_000,
            )
            .unwrap();
            third_completions.lock().unwrap().push("third".to_owned());
            response
        });

        let cancel = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            "/v1/requests/req-first/cancel",
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        assert_eq!(cancel.status_code, 200);

        let first = first.join().unwrap();
        assert_eq!(first.status_code, 409);
        let first_body: Value = serde_json::from_slice(&first.body).unwrap();
        assert_eq!(first_body["error"]["code"], "cancelled");
        let second = second.join().unwrap();
        let third = third.join().unwrap();
        assert_eq!(second.status_code, 200);
        assert_eq!(third.status_code, 200);
        assert_eq!(completions.lock().unwrap().as_slice(), ["second", "third"]);

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_cancel_queued_request() {
        let _guard = integration_lock();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.research_timeout_ms = Some(5_000);
        }
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let first_body = serde_json::to_vec(&json!({
            "model": "tablinq-research",
            "messages": [{ "role": "user", "content": "occupy the only slot" }]
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
                    ("x-request-id", "req-running"),
                ],
                &first_body,
                20_000,
            )
        });

        thread::sleep(Duration::from_millis(250));

        let queued_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "cancel me while queued" }]
        }))
        .unwrap();
        let host = snapshot.host.clone();
        let auth_for_thread = auth.clone();
        let queued = thread::spawn(move || {
            http_request(
                &host,
                port,
                "POST",
                V1_CHAT_COMPLETIONS_PATH,
                &[
                    ("authorization", auth_for_thread.as_str()),
                    ("content-type", "application/json"),
                    ("x-request-id", "req-cancel-me"),
                ],
                &queued_body,
                20_000,
            )
            .unwrap()
        });

        thread::sleep(Duration::from_millis(50));

        let cancel = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            "/v1/requests/req-cancel-me/cancel",
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        assert_eq!(cancel.status_code, 200);
        let cancel_body: Value = serde_json::from_slice(&cancel.body).unwrap();
        assert_eq!(cancel_body["accepted"], true);
        assert_eq!(cancel_body["state"], "queued");

        let queued = queued.join().unwrap();
        assert_eq!(queued.status_code, 409);
        let queued_body: Value = serde_json::from_slice(&queued.body).unwrap();
        assert_eq!(queued_body["error"]["code"], "cancelled");

        let cancel_running = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            "/v1/requests/req-running/cancel",
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        assert_eq!(cancel_running.status_code, 200);

        let first = first.join().unwrap().unwrap();
        assert_eq!(first.status_code, 409);
        let first_body: Value = serde_json::from_slice(&first.body).unwrap();
        assert_eq!(first_body["error"]["code"], "cancelled");

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_cancel_running_research_request() {
        let _guard = integration_lock();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.research_timeout_ms = Some(10_000);
        }
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let body = serde_json::to_vec(&json!({
            "model": "tablinq-research",
            "messages": [{ "role": "user", "content": "cancel a running research job" }]
        }))
        .unwrap();
        let host = snapshot.host.clone();
        let port = snapshot.port;
        let auth_for_thread = auth.clone();
        let running = thread::spawn(move || {
            http_request(
                &host,
                port,
                "POST",
                V1_CHAT_COMPLETIONS_PATH,
                &[
                    ("authorization", auth_for_thread.as_str()),
                    ("content-type", "application/json"),
                    ("x-request-id", "req-running-cancel"),
                ],
                &body,
                20_000,
            )
            .unwrap()
        });

        thread::sleep(Duration::from_millis(500));

        let cancel = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            "/v1/requests/req-running-cancel/cancel",
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        assert_eq!(cancel.status_code, 200);
        let cancel_body: Value = serde_json::from_slice(&cancel.body).unwrap();
        assert_eq!(cancel_body["accepted"], true);
        assert_eq!(cancel_body["state"], "running");

        let running = running.join().unwrap();
        assert_eq!(running.status_code, 409);
        let body: Value = serde_json::from_slice(&running.body).unwrap();
        assert_eq!(body["error"]["code"], "cancelled");

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_return_queue_timeout_when_wait_expires() {
        let _guard = integration_lock();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.research_timeout_ms = Some(5_000);
            gateway.queue_timeout_ms = Some(250);
        }
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let first_body = serde_json::to_vec(&json!({
            "model": "tablinq-research",
            "messages": [{ "role": "user", "content": "occupy the only slot" }]
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

        thread::sleep(Duration::from_millis(150));

        let second_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "time out in queue" }]
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
        assert_eq!(body["error"]["code"], "queue_timeout");

        let first = first.join().unwrap().unwrap();
        assert_eq!(first.status_code, 200);

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn capability_cache_returns_cached_catalog_for_profile_workspace_key() {
        let _guard = integration_lock();
        let dir = tempdir().unwrap();
        let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        let cache = CapabilityCatalogCache::load(&config).unwrap();
        let report = CapabilityReport {
            reported_at: Utc::now(),
            selector_pack_id: "chatgpt-default".to_owned(),
            selector_pack_version: "v1".to_owned(),
            matrix: shared_protocol::CapabilityMatrix {
                composer_ready: true,
                submit_available: true,
                model_picker_visible: true,
                thinking_effort_visible: true,
                search_mode_visible: true,
                research_mode_visible: true,
                upload_control_visible: true,
                downloadable_artifacts_observable: true,
            },
            aliases: shared_protocol::RuntimeAliases {
                modes: BTreeMap::from([
                    ("normal".to_owned(), vec!["Chat".to_owned()]),
                    ("search".to_owned(), vec!["Browse the web".to_owned()]),
                    ("research".to_owned(), vec!["Deep research".to_owned()]),
                ]),
                model_classes: BTreeMap::from([
                    ("instant".to_owned(), vec!["Instant answers".to_owned()]),
                    ("thinking".to_owned(), vec!["Deep thinking".to_owned()]),
                ]),
                efforts: BTreeMap::from([
                    ("standard".to_owned(), vec!["Standard reasoning".to_owned()]),
                    ("extended".to_owned(), vec!["Extended reasoning".to_owned()]),
                ]),
            },
        };

        cache
            .remember(
                "profile-a::workspace-a",
                "profile-a",
                "workspace-a",
                &report,
            )
            .unwrap();
        let cached = cache.cached("profile-a::workspace-a").unwrap().unwrap();
        assert_eq!(cached.profile_dir, "profile-a");
        assert_eq!(cached.workspace_key, "workspace-a");
        assert_eq!(
            cached.report.aliases.model_classes["thinking"],
            vec!["Deep thinking".to_owned()]
        );
    }
}
