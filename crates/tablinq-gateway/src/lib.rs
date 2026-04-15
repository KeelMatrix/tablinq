pub mod build_info;
mod openai;

use crate::openai::{
    ChatCompletionsRequest, PROVIDER_MODELS, RequestRole, model_object, openai_error,
    provider_capability_catalog, request_role_name, resolve_model,
};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use gptd::core::{PreparedAskJob, PreparedResearchJob, error_code_for_message};
use gptd::logging::JsonLogger;
use gptd::{AppConfig, CoreService};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use shared_protocol::{
    ArtifactSummary, BrowserOpenRequest, BrowserStatusSnapshot, CapabilityReport, ErrorCode,
    EventPayload, GatewayRequestContext, JobStatus, RequestExecutionKind, RequestSummary,
    RequestUsage, StreamChunkKind,
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
const V1_USAGE_PATH: &str = "/v1/usage";
const V1_REQUESTS_PREFIX: &str = "/v1/requests/";
const STAGED_UPLOAD_RETENTION_SECS: i64 = 24 * 60 * 60;
const REQUEST_SUMMARY_LIMIT: usize = 50;

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

struct GatewayRuntimeGuard {
    status_path: PathBuf,
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
struct StagedFileMetadata {
    file_id: String,
    file_name: String,
    content_type: String,
    size_bytes: u64,
    sha256: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    staged_path: String,
    #[serde(default)]
    linked_request_ids: Vec<String>,
    #[serde(default)]
    linked_conversation_ids: Vec<String>,
}

#[derive(Debug)]
struct UploadStore {
    root: PathBuf,
    retention_secs: i64,
    next_id: AtomicU64,
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
            if let Some(entry) = state.requests.get(request_id)
                && entry.cancelled
            {
                remove_queued_request(&mut state, request_id);
                self.changed.notify_all();
                bail!("request cancelled while queued");
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

impl UploadStore {
    fn load(config: &AppConfig) -> Result<Self> {
        let root = gateway_uploads_dir(config);
        fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
        let store = Self {
            root,
            retention_secs: STAGED_UPLOAD_RETENTION_SECS,
            next_id: AtomicU64::new(0),
        };
        store.cleanup_expired()?;
        Ok(store)
    }

    fn stage_upload(
        &self,
        file_name: &str,
        content_type: Option<&str>,
        body: &[u8],
    ) -> Result<StagedFileMetadata> {
        self.cleanup_expired()?;

        let safe_name = sanitize_upload_file_name(file_name);
        let created_at = Utc::now();
        let file_id = format!(
            "file-{}-{}",
            created_at.timestamp_millis(),
            self.next_id.fetch_add(1, Ordering::SeqCst)
        );
        let dir = self.root.join(&file_id);
        fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;

        let staged_path = dir.join(&safe_name);
        fs::write(&staged_path, body)
            .with_context(|| format!("write {}", staged_path.display()))?;

        let metadata = StagedFileMetadata {
            file_id: file_id.clone(),
            file_name: safe_name,
            content_type: normalize_content_type(content_type),
            size_bytes: body.len() as u64,
            sha256: sha256_hex(body),
            created_at,
            expires_at: created_at + chrono::Duration::seconds(self.retention_secs),
            staged_path: staged_path.display().to_string(),
            linked_request_ids: vec![],
            linked_conversation_ids: vec![],
        };
        self.write_metadata(&dir, &metadata)?;
        Ok(metadata)
    }

    fn resolve_file_inputs(
        &self,
        file_paths: Vec<String>,
        file_ids: &[String],
    ) -> Result<Vec<String>> {
        self.cleanup_expired()?;
        let mut resolved = file_paths;
        for file_id in file_ids {
            let metadata = self
                .get(file_id)?
                .with_context(|| format!("staged file {file_id} was not found"))?;
            resolved.push(metadata.staged_path);
        }
        Ok(resolved)
    }

    fn get(&self, file_id: &str) -> Result<Option<StagedFileMetadata>> {
        self.cleanup_expired()?;
        let dir = self.root.join(file_id);
        if !dir.exists() {
            return Ok(None);
        }
        let metadata = read_staged_file_metadata(&dir)?;
        if !Path::new(&metadata.staged_path).exists() {
            let _ = fs::remove_dir_all(&dir);
            return Ok(None);
        }
        Ok(Some(metadata))
    }

    fn list(&self) -> Result<Vec<StagedFileMetadata>> {
        self.cleanup_expired()?;
        let mut items = Vec::new();
        for entry in
            fs::read_dir(&self.root).with_context(|| format!("read {}", self.root.display()))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            if let Ok(metadata) = read_staged_file_metadata(&entry.path())
                && Path::new(&metadata.staged_path).exists()
            {
                items.push(metadata);
            }
        }
        items.sort_by(|left, right| right.created_at.cmp(&left.created_at));
        Ok(items)
    }

    fn mark_linked_usage(
        &self,
        file_ids: &[String],
        request_id: &str,
        conversation_id: Option<&str>,
    ) -> Result<()> {
        if file_ids.is_empty() {
            return Ok(());
        }

        self.cleanup_expired()?;
        for file_id in file_ids {
            let dir = self.root.join(file_id);
            if !dir.exists() {
                continue;
            }
            let mut metadata = match read_staged_file_metadata(&dir) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            remember_link(&mut metadata.linked_request_ids, request_id);
            if let Some(conversation_id) = conversation_id {
                remember_link(&mut metadata.linked_conversation_ids, conversation_id);
            }
            self.write_metadata(&dir, &metadata)?;
        }
        Ok(())
    }

    fn cleanup_expired(&self) -> Result<()> {
        let now = Utc::now();
        for entry in
            fs::read_dir(&self.root).with_context(|| format!("read {}", self.root.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_dir() {
                let _ = fs::remove_file(path);
                continue;
            }

            let expired_or_invalid = match read_staged_file_metadata(&path) {
                Ok(metadata) => {
                    metadata.expires_at <= now || !Path::new(&metadata.staged_path).exists()
                }
                Err(_) => true,
            };
            if expired_or_invalid {
                let _ = fs::remove_dir_all(&path);
            }
        }
        Ok(())
    }

    fn write_metadata(&self, dir: &Path, metadata: &StagedFileMetadata) -> Result<()> {
        let path = staged_metadata_path(dir);
        let raw = serde_json::to_vec_pretty(metadata).context("encode staged upload metadata")?;
        fs::write(&path, raw).with_context(|| format!("write {}", path.display()))
    }
}

fn read_staged_file_metadata(dir: &Path) -> Result<StagedFileMetadata> {
    let path = staged_metadata_path(dir);
    let raw = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&raw).context("decode staged upload metadata")
}

fn staged_metadata_path(dir: &Path) -> PathBuf {
    dir.join("metadata.json")
}

fn gateway_uploads_dir(config: &AppConfig) -> PathBuf {
    config.paths.state_root.join("gateway-uploads")
}

fn sanitize_upload_file_name(file_name: &str) -> String {
    let candidate = file_name
        .rsplit(['/', '\\'])
        .next()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("upload.bin");
    let mut sanitized = candidate
        .chars()
        .map(|ch| {
            if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || ch.is_control()
            {
                '_'
            } else {
                ch
            }
        })
        .collect::<String>()
        .trim()
        .trim_end_matches(['.', ' '])
        .to_owned();
    if sanitized.is_empty() {
        sanitized = "upload.bin".to_owned();
    }
    sanitized
}

fn remember_link(items: &mut Vec<String>, value: &str) {
    if value.trim().is_empty() || items.iter().any(|item| item == value) {
        return;
    }
    items.push(value.to_owned());
    items.sort();
}

fn normalize_content_type(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("application/octet-stream")
        .to_owned()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut text, "{byte:02x}");
    }
    text
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

fn file_id_from_path(path: &str) -> Option<&str> {
    path.strip_prefix(&format!("{V1_FILES_PATH}/"))
        .filter(|value| !value.contains('/'))
}

fn artifact_id_from_path(path: &str) -> Option<(&str, bool)> {
    let suffix = path.strip_prefix(&format!("{V1_ARTIFACTS_PATH}/"))?;
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

enum PreparedChatJob {
    Ask(PreparedAskJob),
    Research(PreparedResearchJob),
}

struct PreparedChatRequest {
    request: ChatCompletionsRequest,
    model_id: &'static str,
    request_role: RequestRole,
    parent_request_id: Option<String>,
    file_ids: Vec<String>,
    job: PreparedChatJob,
}

impl PreparedChatRequest {
    fn job_id(&self) -> &str {
        match &self.job {
            PreparedChatJob::Ask(job) => &job.job_id,
            PreparedChatJob::Research(job) => &job.job_id,
        }
    }

    fn mode(&self) -> shared_protocol::Mode {
        match &self.job {
            PreparedChatJob::Ask(job) => job.command.mode,
            PreparedChatJob::Research(_) => shared_protocol::Mode::Research,
        }
    }

    fn conversation_id_hint(&self) -> Option<&str> {
        match &self.job {
            PreparedChatJob::Ask(job) => job.command.conversation_id.as_deref(),
            PreparedChatJob::Research(job) => job.command.conversation_id.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestOutcomeKind {
    Completed,
    Recovered,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
struct RequestOutcome {
    kind: RequestOutcomeKind,
    conversation_id: Option<String>,
    interrupted_at: Option<DateTime<Utc>>,
    interruption_reason: Option<String>,
    recovered_at: Option<DateTime<Utc>>,
    recovery_reason: Option<String>,
    failed_at: Option<DateTime<Utc>>,
    failure_reason: Option<String>,
}

fn prepare_chat_completions_request(
    core: &CoreService,
    upload_store: &UploadStore,
    context: &RequestContext,
) -> Result<PreparedChatRequest> {
    let request: ChatCompletionsRequest =
        serde_json::from_slice(&context.request.body).context("decode chat completions request")?;
    openai::validate_chat_request(&request)?;
    let model = resolve_model(&request.model).context("resolve provider model")?;
    let request_role = request.request_role();
    let parent_request_id = request.tablinq_parent_request_id.clone();
    let flattened = openai::flatten_messages(&request)?;
    let files = upload_store.resolve_file_inputs(flattened.file_paths, &flattened.file_ids)?;
    let conversation_id = request
        .tablinq_conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let new_conversation = request.tablinq_new_conversation;

    let job = if model.uses_research {
        PreparedChatJob::Research(core.prepare_research(shared_protocol::ResearchCommand {
            query: flattened.prompt,
            conversation_id,
            new_conversation,
            files,
            output_dir: None,
        })?)
    } else {
        PreparedChatJob::Ask(core.prepare_ask(shared_protocol::AskCommand {
            prompt: flattened.prompt,
            conversation_id,
            new_conversation,
            mode: model.mode,
            model_class: model.model_class,
            effort: model.effort,
            files,
        })?)
    };

    Ok(PreparedChatRequest {
        request,
        model_id: model.id,
        request_role,
        parent_request_id,
        file_ids: flattened.file_ids,
        job,
    })
}

fn execute_chat_completions_request(
    stream: &mut TcpStream,
    core: &CoreService,
    context: &RequestContext,
    logger: &JsonLogger,
    prepared: &PreparedChatRequest,
    client_id: &str,
) -> Result<RequestOutcome> {
    ensure_browser_session(core)?;

    if prepared.request.stream {
        let stream_id = format!("chatcmpl-{}", Utc::now().timestamp_millis());
        let mut downstream_interrupted: Option<(DateTime<Utc>, String)> = None;
        if let Err(err) = write_sse_headers(stream, &context.request_id) {
            if is_client_disconnect_error(&err) {
                let interrupted_at = Utc::now();
                let message =
                    format!("gateway client disconnected before streaming completed: {err:#}");
                let _ = core.mark_job_interrupted(
                    prepared.job_id(),
                    prepared.conversation_id_hint(),
                    &message,
                );
                downstream_interrupted = Some((interrupted_at, message));
            } else {
                return Err(err);
            }
        }
        if downstream_interrupted.is_none()
            && let Err(err) = write_sse_event(
                stream,
                &json!({
                    "id": stream_id,
                    "object": "chat.completion.chunk",
                    "created": Utc::now().timestamp(),
                    "model": prepared.model_id,
                    "request_id": context.request_id,
                    "tablinq_job_id": prepared.job_id(),
                    "tablinq_request_role": request_role_name(prepared.request_role),
                    "tablinq_parent_request_id": prepared.parent_request_id.as_deref(),
                    "choices": [{
                        "index": 0,
                        "delta": { "role": "assistant" },
                        "finish_reason": Value::Null,
                    }],
                }),
            )
        {
            if is_client_disconnect_error(&err) {
                let interrupted_at = Utc::now();
                let message =
                    format!("gateway client disconnected before streaming completed: {err:#}");
                let _ = core.mark_job_interrupted(
                    prepared.job_id(),
                    prepared.conversation_id_hint(),
                    &message,
                );
                downstream_interrupted = Some((interrupted_at, message));
            } else {
                return Err(err);
            }
        }

        let result = match &prepared.job {
            PreparedChatJob::Research(job) => core
                .execute_prepared_research(job, |event| {
                    if downstream_interrupted.is_some() {
                        return Ok(());
                    }
                    if let Err(err) = write_stream_delta(
                        stream,
                        &stream_id,
                        prepared.model_id,
                        &context.request_id,
                        event.clone(),
                    ) {
                        if is_client_disconnect_error(&err) {
                            let interrupted_at = Utc::now();
                            let message = format!(
                                "gateway client disconnected before streaming completed: {err:#}"
                            );
                            let _ = core.mark_job_interrupted(
                                prepared.job_id(),
                                event_conversation_id(&event).as_deref(),
                                &message,
                            );
                            downstream_interrupted = Some((interrupted_at, message));
                            return Ok(());
                        }
                        return Err(err);
                    }
                    Ok(())
                })
                .map(|result| result.conversation_id),
            PreparedChatJob::Ask(job) => core
                .execute_prepared_ask(job, |event| {
                    if downstream_interrupted.is_some() {
                        return Ok(());
                    }
                    if let Err(err) = write_stream_delta(
                        stream,
                        &stream_id,
                        prepared.model_id,
                        &context.request_id,
                        event.clone(),
                    ) {
                        if is_client_disconnect_error(&err) {
                            let interrupted_at = Utc::now();
                            let message = format!(
                                "gateway client disconnected before streaming completed: {err:#}"
                            );
                            let _ = core.mark_job_interrupted(
                                prepared.job_id(),
                                event_conversation_id(&event).as_deref(),
                                &message,
                            );
                            downstream_interrupted = Some((interrupted_at, message));
                            return Ok(());
                        }
                        return Err(err);
                    }
                    Ok(())
                })
                .map(|result| result.conversation_id),
        };

        match result {
            Ok(conversation_id) => {
                if let Some((interrupted_at, interruption_reason)) = downstream_interrupted.take() {
                    let recovered_at = Utc::now();
                    let recovery_reason =
                        "gateway client disconnected, but the upstream request completed"
                            .to_owned();
                    let _ = core.mark_job_recovered(
                        prepared.job_id(),
                        Some(&conversation_id),
                        &recovery_reason,
                    );
                    logger.info(
                        "gateway recovered interrupted client stream",
                        json!({
                            "request_id": context.request_id,
                            "job_id": prepared.job_id(),
                        }),
                    )?;
                    return Ok(RequestOutcome {
                        kind: RequestOutcomeKind::Recovered,
                        conversation_id: Some(conversation_id),
                        interrupted_at: Some(interrupted_at),
                        interruption_reason: Some(interruption_reason),
                        recovered_at: Some(recovered_at),
                        recovery_reason: Some(recovery_reason),
                        failed_at: None,
                        failure_reason: None,
                    });
                }

                if let Err(err) = write_sse_event(
                    stream,
                    &json!({
                        "id": stream_id,
                        "object": "chat.completion.chunk",
                        "created": Utc::now().timestamp(),
                        "model": prepared.model_id,
                        "request_id": context.request_id,
                        "tablinq_job_id": prepared.job_id(),
                        "tablinq_request_role": request_role_name(prepared.request_role),
                        "tablinq_parent_request_id": prepared.parent_request_id.as_deref(),
                        "choices": [{
                            "index": 0,
                            "delta": {},
                            "finish_reason": "stop",
                        }],
                    }),
                ) {
                    if is_client_disconnect_error(&err) {
                        let interrupted_at = Utc::now();
                        let interruption_reason = format!(
                            "gateway client disconnected before streaming completed: {err:#}"
                        );
                        let recovered_at = Utc::now();
                        let recovery_reason =
                            "gateway client disconnected, but the upstream request completed"
                                .to_owned();
                        let _ = core.mark_job_interrupted(
                            prepared.job_id(),
                            Some(&conversation_id),
                            &interruption_reason,
                        );
                        let _ = core.mark_job_recovered(
                            prepared.job_id(),
                            Some(&conversation_id),
                            &recovery_reason,
                        );
                        return Ok(RequestOutcome {
                            kind: RequestOutcomeKind::Recovered,
                            conversation_id: Some(conversation_id),
                            interrupted_at: Some(interrupted_at),
                            interruption_reason: Some(interruption_reason),
                            recovered_at: Some(recovered_at),
                            recovery_reason: Some(recovery_reason),
                            failed_at: None,
                            failure_reason: None,
                        });
                    }
                    return Err(err);
                }
                if let Err(err) = write_sse_done(stream) {
                    if is_client_disconnect_error(&err) {
                        let interrupted_at = Utc::now();
                        let interruption_reason = format!(
                            "gateway client disconnected before streaming completed: {err:#}"
                        );
                        let recovered_at = Utc::now();
                        let recovery_reason =
                            "gateway client disconnected, but the upstream request completed"
                                .to_owned();
                        let _ = core.mark_job_interrupted(
                            prepared.job_id(),
                            Some(&conversation_id),
                            &interruption_reason,
                        );
                        let _ = core.mark_job_recovered(
                            prepared.job_id(),
                            Some(&conversation_id),
                            &recovery_reason,
                        );
                        return Ok(RequestOutcome {
                            kind: RequestOutcomeKind::Recovered,
                            conversation_id: Some(conversation_id),
                            interrupted_at: Some(interrupted_at),
                            interruption_reason: Some(interruption_reason),
                            recovered_at: Some(recovered_at),
                            recovery_reason: Some(recovery_reason),
                            failed_at: None,
                            failure_reason: None,
                        });
                    }
                    return Err(err);
                }
                logger.info(
                    "gateway streamed chat completion",
                    json!({
                        "request_id": context.request_id,
                        "model": prepared.model_id,
                        "job_id": prepared.job_id(),
                    }),
                )?;
                Ok(RequestOutcome {
                    kind: RequestOutcomeKind::Completed,
                    conversation_id: Some(conversation_id),
                    interrupted_at: None,
                    interruption_reason: None,
                    recovered_at: None,
                    recovery_reason: None,
                    failed_at: None,
                    failure_reason: None,
                })
            }
            Err(err) => {
                let failure_bundle_id =
                    capture_gateway_failure_bundle(core, context, prepared, client_id, &err);
                if downstream_interrupted.is_none() {
                    write_sse_event(
                        stream,
                        &map_openai_error_value(
                            &context.request_id,
                            &err,
                            failure_bundle_id.as_deref(),
                        ),
                    )?;
                    write_sse_done(stream)?;
                }
                let mut outcome = outcome_from_error(&err);
                if let Some((interrupted_at, interruption_reason)) = downstream_interrupted {
                    outcome.interrupted_at = Some(interrupted_at);
                    outcome.interruption_reason = Some(interruption_reason);
                }
                Ok(outcome)
            }
        }
    } else {
        let (response, conversation_id) = match &prepared.job {
            PreparedChatJob::Research(job) => {
                let result = core.execute_prepared_research(job, |_event| Ok(()))?;
                let conversation = core.find_conversation(&result.conversation_id)?;
                (
                    build_chat_completion_response(
                        &context.request_id,
                        prepared.job_id(),
                        prepared.model_id,
                        prepared.request_role,
                        prepared.parent_request_id.as_deref(),
                        Some(&result.conversation_id),
                        conversation.as_ref().map(|item| item.session_id.as_str()),
                        &result.report_text,
                        "stop",
                    ),
                    Some(result.conversation_id),
                )
            }
            PreparedChatJob::Ask(job) => {
                let result = core.execute_prepared_ask(job, |_event| Ok(()))?;
                let conversation = core.find_conversation(&result.conversation_id)?;
                (
                    build_chat_completion_response(
                        &context.request_id,
                        prepared.job_id(),
                        prepared.model_id,
                        prepared.request_role,
                        prepared.parent_request_id.as_deref(),
                        Some(&result.conversation_id),
                        conversation.as_ref().map(|item| item.session_id.as_str()),
                        &result.output_text,
                        "stop",
                    ),
                    Some(result.conversation_id),
                )
            }
        };
        log_response(logger, &context.request_id, response.status_code)?;
        if let Err(err) = write_http_response(stream, &context.request_id, response) {
            if is_client_disconnect_error(&err) {
                let interrupted_at = Utc::now();
                let interruption_reason = format!(
                    "gateway client disconnected before the final response was written: {err:#}"
                );
                let recovered_at = Utc::now();
                let recovery_reason =
                    "gateway client disconnected, but the upstream request completed".to_owned();
                let _ = core.mark_job_interrupted(
                    prepared.job_id(),
                    conversation_id.as_deref(),
                    &interruption_reason,
                );
                let _ = core.mark_job_recovered(
                    prepared.job_id(),
                    conversation_id.as_deref(),
                    &recovery_reason,
                );
                return Ok(RequestOutcome {
                    kind: RequestOutcomeKind::Recovered,
                    conversation_id,
                    interrupted_at: Some(interrupted_at),
                    interruption_reason: Some(interruption_reason),
                    recovered_at: Some(recovered_at),
                    recovery_reason: Some(recovery_reason),
                    failed_at: None,
                    failure_reason: None,
                });
            }
            return Err(err);
        }
        Ok(RequestOutcome {
            kind: RequestOutcomeKind::Completed,
            conversation_id,
            interrupted_at: None,
            interruption_reason: None,
            recovered_at: None,
            recovery_reason: None,
            failed_at: None,
            failure_reason: None,
        })
    }
}

fn build_request_summary(
    context: &RequestContext,
    prepared: &PreparedChatRequest,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    queue_wait_ms: u64,
    outcome: RequestOutcome,
) -> RequestSummary {
    RequestSummary {
        request_id: context.request_id.clone(),
        job_id: prepared.job_id().to_owned(),
        model_id: Some(prepared.model_id.to_owned()),
        request_role: Some(request_role_name(prepared.request_role).to_owned()),
        parent_request_id: prepared.parent_request_id.clone(),
        execution_kind: match &prepared.job {
            PreparedChatJob::Ask(_) => RequestExecutionKind::Ask,
            PreparedChatJob::Research(_) => RequestExecutionKind::Research,
        },
        status: match outcome.kind {
            RequestOutcomeKind::Completed => JobStatus::Completed,
            RequestOutcomeKind::Recovered => JobStatus::Recovered,
            RequestOutcomeKind::Failed => JobStatus::Failed,
            RequestOutcomeKind::Cancelled => JobStatus::Cancelled,
        },
        cancelled: outcome.kind == RequestOutcomeKind::Cancelled,
        start_time,
        end_time: Some(end_time),
        duration_ms: Some((end_time - start_time).num_milliseconds().max(0) as u64),
        queue_wait_ms,
        interrupted_at: outcome.interrupted_at,
        interruption_reason: outcome.interruption_reason,
        recovered_at: outcome.recovered_at,
        recovery_reason: outcome.recovery_reason,
        failed_at: outcome.failed_at,
        failure_reason: outcome.failure_reason,
        usage: RequestUsage {
            status: "unknown".to_owned(),
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
        },
        warnings: vec![
            "exact token usage is unavailable for the browser-backed provider".to_owned(),
        ],
    }
}

fn outcome_from_error(err: &anyhow::Error) -> RequestOutcome {
    let message = format!("{err:#}");
    RequestOutcome {
        kind: if message.contains("cancelled") {
            RequestOutcomeKind::Cancelled
        } else {
            RequestOutcomeKind::Failed
        },
        conversation_id: None,
        interrupted_at: None,
        interruption_reason: None,
        recovered_at: None,
        recovery_reason: None,
        failed_at: if message.contains("cancelled") {
            None
        } else {
            Some(Utc::now())
        },
        failure_reason: if message.contains("cancelled") {
            None
        } else {
            Some(message)
        },
    }
}

fn is_client_disconnect_error(err: &anyhow::Error) -> bool {
    let message = format!("{err:#}").to_ascii_lowercase();
    message.contains("broken pipe")
        || message.contains("connection reset")
        || message.contains("forcibly closed")
        || message.contains("connection aborted")
}

fn event_conversation_id(event: &EventPayload) -> Option<String> {
    match event {
        EventPayload::StreamChunk(chunk) => chunk.conversation_id.clone(),
        EventPayload::JobState(state) => state.conversation_id.clone(),
        EventPayload::ArtifactReady(artifact) => artifact.conversation_id.clone(),
        _ => None,
    }
}

fn write_stream_delta(
    stream: &mut TcpStream,
    stream_id: &str,
    model_id: &str,
    request_id: &str,
    event: EventPayload,
) -> Result<()> {
    if let EventPayload::StreamChunk(chunk) = event
        && chunk.kind == StreamChunkKind::Delta
        && !chunk.text.is_empty()
    {
        write_sse_event(
            stream,
            &json!({
                "id": stream_id,
                "object": "chat.completion.chunk",
                "created": Utc::now().timestamp(),
                "model": model_id,
                "request_id": request_id,
                "tablinq_conversation_id": chunk.conversation_id,
                "choices": [{
                    "index": 0,
                    "delta": { "content": chunk.text },
                    "finish_reason": Value::Null,
                }],
            }),
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_chat_completion_response(
    request_id: &str,
    job_id: &str,
    model_id: &str,
    request_role: RequestRole,
    parent_request_id: Option<&str>,
    conversation_id: Option<&str>,
    session_id: Option<&str>,
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
            "tablinq_request_role": request_role_name(request_role),
            "tablinq_parent_request_id": parent_request_id,
            "tablinq_conversation_id": conversation_id,
            "tablinq_session_id": session_id,
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
            "message": ack.message,
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

fn request_id_from_path(path: &str) -> Option<&str> {
    let suffix = path.strip_prefix(V1_REQUESTS_PREFIX)?;
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
    if let Ok(snapshot) = core.browser_profile_info()
        && !snapshot.running
    {
        let _ = core.browser_open(BrowserOpenRequest {
            target_url: None,
            browser_path: None,
            headless: Some(true),
        });
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
    let startup_probe_timeout_ms = config
        .gateway_request_timeout_ms()
        .min(STARTUP_PROBE_TIMEOUT_MS);
    loop {
        if let Some(snapshot) = load_status_snapshot(&config)?
            && gateway_is_ready(&snapshot, startup_probe_timeout_ms)
        {
            return Ok(json!({
                "status": "started",
                "host": snapshot.host,
                "port": snapshot.port,
                "pid": snapshot.pid,
            }));
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
            "log_path": snapshot.log_path,
        }))
    } else {
        Ok(json!({
            "running": false,
            "ready": false,
            "log_path": gateway_log_path(&config).display().to_string(),
        }))
    }
}

pub fn gateway_doctor(state_root: Option<&Path>) -> Result<Value> {
    let config = AppConfig::load_or_create(state_root)?;
    let Some(snapshot) = load_status_snapshot(&config)? else {
        return Ok(json!({
            "running": false,
            "ready": false,
            "status": Value::Null,
            "health": Value::Null,
            "admin": Value::Null,
            "scheduler": Value::Null,
            "logs": {
                "gateway": gateway_log_path(&config).display().to_string(),
            },
        }));
    };

    let running = gateway_is_responsive(&snapshot, config.gateway_request_timeout_ms());
    let ready = gateway_is_ready(&snapshot, config.gateway_request_timeout_ms());
    let health = gateway_json_request(
        &snapshot,
        HEALTH_PATH,
        None,
        config.gateway_request_timeout_ms(),
    )
    .ok();
    let admin = gateway_json_request(
        &snapshot,
        ADMIN_STATUS_PATH,
        config.gateway_api_key(),
        config.gateway_request_timeout_ms(),
    )
    .ok();

    Ok(json!({
        "running": running,
        "ready": ready,
        "status": snapshot,
        "health": health,
        "admin": admin,
        "scheduler": admin.as_ref().and_then(|value| value.get("scheduler")).cloned(),
        "logs": {
            "gateway": gateway_log_path(&config).display().to_string(),
        },
    }))
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
        if let Some(snapshot) = load_status_snapshot(config)?
            && gateway_is_responsive(&snapshot, config.gateway_request_timeout_ms())
        {
            bail!("gateway already running");
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

impl Drop for GatewayRuntimeGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.status_path);
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

fn gateway_json_request(
    snapshot: &GatewayStatusSnapshot,
    path: &str,
    api_key: Option<&str>,
    timeout_ms: u64,
) -> Result<Value> {
    let auth = api_key
        .map(|value| format!("Bearer {value}"))
        .unwrap_or_default();
    let mut headers = Vec::new();
    if !auth.is_empty() {
        headers.push(("authorization", auth.as_str()));
    }
    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        path,
        &headers,
        &[],
        timeout_ms,
    )?;
    if response.status_code != 200 {
        bail!(
            "gateway request {path} returned HTTP {}",
            response.status_code
        );
    }
    serde_json::from_slice(&response.body).context("decode gateway JSON response")
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
    use gptd::Database;
    use gptd::browser::browser_integration_prereqs_available;
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

    macro_rules! require_browser_integration_prereqs {
        () => {
            if !browser_integration_prereqs_available() {
                eprintln!(
                    "skipping browser-backed gateway test: browser adapter prerequisites are unavailable"
                );
                return;
            }
        };
    }

    struct TestChild {
        child: Child,
    }

    impl TestChild {
        fn spawn(mut command: Command) -> Self {
            let child = command.spawn().expect("spawn test child");
            Self { child }
        }
    }

    impl std::ops::Deref for TestChild {
        type Target = Child;

        fn deref(&self) -> &Self::Target {
            &self.child
        }
    }

    impl std::ops::DerefMut for TestChild {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.child
        }
    }

    impl Drop for TestChild {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn spawn_fake_chatgpt() -> (TestChild, String) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("allocate tcp port");
        let port = listener.local_addr().expect("resolve local address").port();
        drop(listener);

        let server_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("packages")
            .join("fake-chatgpt")
            .join("server.mjs");
        let mut command = Command::new("node");
        command
            .arg(server_path)
            .env("PORT", port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let child = TestChild::spawn(command);

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
        config.file.browser_connection_mode =
            Some(shared_protocol::BrowserConnectionMode::ManagedOnly);
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

    fn upload_file(
        snapshot: &GatewayStatusSnapshot,
        auth: &str,
        file_name: &str,
        content_type: &str,
        body: &[u8],
    ) -> SimpleHttpResponse {
        http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_FILES_PATH,
            &[
                ("authorization", auth),
                ("x-file-name", file_name),
                ("content-type", content_type),
            ],
            body,
            20_000,
        )
        .unwrap()
    }

    fn openclaude_request_body(
        model: &str,
        prompt: &str,
        role: &str,
        stream: bool,
        parent_request_id: Option<&str>,
    ) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "model": model,
            "stream": stream,
            "tablinq_request_role": role,
            "tablinq_parent_request_id": parent_request_id,
            "messages": [
                { "role": "user", "content": prompt }
            ]
        }))
        .unwrap()
    }

    fn paperclip_request_body(
        model: &str,
        prompt: &str,
        stream: bool,
        file_ids: &[&str],
    ) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "model": model,
            "stream": stream,
            "tablinq_file_ids": file_ids,
            "messages": [
                { "role": "user", "content": prompt }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn upload_store_cleans_up_expired_staged_files() {
        let dir = tempdir().unwrap();
        let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        let store = UploadStore::load(&config).unwrap();
        let metadata = store
            .stage_upload("..\\unsafe?.txt", Some("text/plain"), b"hello")
            .unwrap();
        assert_eq!(metadata.file_name, "unsafe_.txt");

        let upload_dir = gateway_uploads_dir(&config).join(&metadata.file_id);
        let expired = StagedFileMetadata {
            expires_at: Utc::now() - chrono::Duration::seconds(1),
            ..metadata
        };
        fs::write(
            staged_metadata_path(&upload_dir),
            serde_json::to_vec_pretty(&expired).unwrap(),
        )
        .unwrap();

        store.cleanup_expired().unwrap();
        assert!(!upload_dir.exists());
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
    fn gateway_doctor_helper_reports_scheduler_and_logs() {
        let _guard = integration_lock();
        let (_dir, config) = test_config();
        let state_root = config.paths.state_root.clone();
        let (handle, _snapshot) = spawn_gateway(config);

        let doctor = gateway_doctor(Some(&state_root)).unwrap();
        assert_eq!(doctor["running"], true);
        assert_eq!(doctor["ready"], true);
        assert_eq!(doctor["scheduler"]["max_concurrent_upstream_jobs"], 1);
        assert!(
            doctor["logs"]["gateway"]
                .as_str()
                .unwrap()
                .ends_with("gateway.log")
        );

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn provider_endpoint_exposes_openclaude_preset_metadata() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
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
            V1_PROVIDER_PATH,
            &[("authorization", auth.as_str())],
            &[],
            10_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 200);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["object"], "tablinq.provider");
        assert_eq!(body["capability_source"], "live");
        assert_eq!(body["request_annotations"]["field"], "tablinq_request_role");
        assert_eq!(
            body["provider_capabilities"]["capabilities"]["search"]["availability"],
            "available"
        );
        assert_eq!(
            body["provider_capabilities"]["capabilities"]["research"]["availability"],
            "available"
        );
        assert_eq!(
            body["provider_capabilities"]["capabilities"]["uploads"]["availability"],
            "available"
        );
        assert_eq!(
            body["provider_capabilities"]["capabilities"]["artifacts"]["availability"],
            "available"
        );
        assert_eq!(
            body["provider_capabilities"]["max_safe_parallelism"]["requests"],
            1
        );
        assert_eq!(
            body["provider_capabilities"]["subagent_recommendation"]["policy"],
            "disabled"
        );
        assert_eq!(body["presets"]["openclaude"]["max_parallel_requests"], 1);
        assert_eq!(
            body["presets"]["openclaude"]["subagents"],
            "disabled_by_default"
        );

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn provider_endpoint_exposes_paperclip_preset_metadata() {
        let _guard = integration_lock();
        let (_dir, config) = test_config();
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            V1_PROVIDER_PATH,
            &[("authorization", auth.as_str())],
            &[],
            10_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 200);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(
            body["presets"]["paperclip"]["transport"],
            "openai_compatible_http"
        );
        assert_eq!(body["presets"]["paperclip"]["max_parallel_requests"], 1);
        assert_eq!(body["presets"]["paperclip"]["files"], true);
        assert_eq!(body["presets"]["paperclip"]["artifacts"], true);
        assert_eq!(
            body["presets"]["paperclip"]["subagent_recommendation"],
            "disable"
        );
        assert_eq!(
            body["presets"]["paperclip"]["usage_accounting"],
            "unknown_or_estimated"
        );

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn documented_provider_contract_surfaces_are_present() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(fake_url);
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let provider = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            V1_PROVIDER_PATH,
            &[("authorization", auth.as_str())],
            &[],
            10_000,
        )
        .unwrap();
        assert_eq!(provider.status_code, 200);
        let provider_body: Value = serde_json::from_slice(&provider.body).unwrap();
        assert_eq!(
            provider_body["request_annotations"]["field"],
            "tablinq_request_role"
        );
        assert_eq!(
            provider_body["request_annotations"]["parent_request_id_field"],
            "tablinq_parent_request_id"
        );
        assert_eq!(
            provider_body["presets"]["openclaude"]["max_parallel_requests"],
            1
        );
        assert_eq!(
            provider_body["presets"]["paperclip"]["max_parallel_requests"],
            1
        );
        assert_eq!(
            provider_body["presets"]["paperclip"]["subagent_recommendation"],
            "disable"
        );

        let chat_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "tablinq_new_conversation": true,
            "messages": [
                { "role": "user", "content": "document the provider contract" }
            ]
        }))
        .unwrap();
        let chat = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
            ],
            &chat_body,
            20_000,
        )
        .unwrap();
        assert_eq!(chat.status_code, 200);
        let chat_body: Value = serde_json::from_slice(&chat.body).unwrap();
        assert!(chat_body["request_id"].as_str().is_some());
        assert!(chat_body["tablinq_conversation_id"].as_str().is_some());
        assert_eq!(chat_body["tablinq_session_id"], "browser-primary");

        let usage = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            V1_USAGE_PATH,
            &[("authorization", auth.as_str())],
            &[],
            10_000,
        )
        .unwrap();
        assert_eq!(usage.status_code, 200);
        let usage_body: Value = serde_json::from_slice(&usage.body).unwrap();
        assert!(
            usage_body["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value.as_str()
                    == Some("token usage is unavailable for the browser-backed provider"))
        );

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn openclaude_primary_streaming_requests_keep_sse_shape() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(fake_url);
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let body =
            openclaude_request_body("tablinq-search", "latest roadmap", "primary", true, None);
        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "oc-primary-1"),
            ],
            &body,
            20_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 200);
        let text = String::from_utf8(response.body).unwrap();
        assert!(text.contains("chat.completion.chunk"));
        assert!(text.contains("\"tablinq_request_role\":\"primary\""));
        assert!(text.contains("[DONE]"));

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn openclaude_subagent_requests_are_rejected_by_default_policy() {
        let _guard = integration_lock();
        let (_dir, config) = test_config();
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let body = openclaude_request_body(
            "tablinq-instant",
            "parallel helper request",
            "subagent",
            false,
            Some("oc-primary-1"),
        );
        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "oc-subagent-1"),
            ],
            &body,
            20_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 429);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["error"]["code"], "subagents_disabled");

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn openclaude_branch_requests_are_serialized_behind_primary_work() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
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

        let primary_body = openclaude_request_body(
            "tablinq-research",
            "hold the browser session",
            "primary",
            false,
            None,
        );
        let host = snapshot.host.clone();
        let port = snapshot.port;
        let auth_for_thread = auth.clone();
        let primary = thread::spawn(move || {
            http_request(
                &host,
                port,
                "POST",
                V1_CHAT_COMPLETIONS_PATH,
                &[
                    ("authorization", auth_for_thread.as_str()),
                    ("content-type", "application/json"),
                    ("x-request-id", "oc-primary-1"),
                ],
                &primary_body,
                20_000,
            )
            .unwrap()
        });

        thread::sleep(Duration::from_millis(250));

        let branch_body = openclaude_request_body(
            "tablinq-instant",
            "branch after primary",
            "branch",
            false,
            Some("oc-primary-1"),
        );
        let host = snapshot.host.clone();
        let auth_for_thread = auth.clone();
        let branch = thread::spawn(move || {
            http_request(
                &host,
                port,
                "POST",
                V1_CHAT_COMPLETIONS_PATH,
                &[
                    ("authorization", auth_for_thread.as_str()),
                    ("content-type", "application/json"),
                    ("x-request-id", "oc-branch-1"),
                ],
                &branch_body,
                20_000,
            )
            .unwrap()
        });

        thread::sleep(Duration::from_millis(150));

        let cancel = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            "/v1/requests/oc-primary-1/cancel",
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        assert_eq!(cancel.status_code, 200);

        let primary = primary.join().unwrap();
        assert_eq!(primary.status_code, 409);
        let branch = branch.join().unwrap();
        assert_eq!(branch.status_code, 200);
        let branch_body: Value = serde_json::from_slice(&branch.body).unwrap();
        assert_eq!(branch_body["tablinq_request_role"], "branch");
        assert_eq!(branch_body["tablinq_parent_request_id"], "oc-primary-1");

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn models_endpoint_returns_openai_model_catalog() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
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
        assert_eq!(
            body["tablinq_model_ids"],
            json!([
                "tablinq-instant",
                "tablinq-thinking-standard",
                "tablinq-thinking-extended",
                "tablinq-search",
                "tablinq-research",
            ])
        );
        let models = body["data"].as_array().unwrap();
        assert_eq!(models.len(), 5);
        assert_eq!(models[0]["id"], "tablinq-instant");
        assert_eq!(models[0]["tablinq_model_class"], "instant");
        assert_eq!(models[0]["tablinq_effort"], "none");
        assert_eq!(models[1]["id"], "tablinq-thinking-standard");
        assert_eq!(models[1]["tablinq_model_class"], "thinking");
        assert_eq!(models[1]["tablinq_effort"], "standard");
        assert_eq!(models[2]["id"], "tablinq-thinking-extended");
        assert_eq!(models[2]["tablinq_effort"], "extended");
        assert_eq!(models[3]["id"], "tablinq-search");
        assert_eq!(models[3]["tablinq_mode"], "search");
        assert_eq!(models[4]["id"], "tablinq-research");
        assert_eq!(models[4]["tablinq_execution_kind"], "research");

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn capabilities_endpoint_reports_alias_remaps_for_stable_models() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
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
        assert_eq!(
            body["provider"]["available_model_ids"],
            json!([
                "tablinq-instant",
                "tablinq-thinking-standard",
                "tablinq-thinking-extended",
                "tablinq-search",
                "tablinq-research",
            ])
        );
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
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url =
            Some(format!("{fake_url}/?missing=effort,search,research"));
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
        let thinking = body["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "tablinq-thinking-standard")
            .unwrap();
        let research = body["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "tablinq-research")
            .unwrap();
        assert_eq!(thinking["tablinq_availability"], "unavailable");
        assert_eq!(search["tablinq_availability"], "unavailable");
        assert_eq!(research["tablinq_availability"], "unavailable");
        assert!(!search["tablinq_warning"].as_array().unwrap().is_empty());
        assert_eq!(
            body["provider"]["capabilities"]["search"]["availability"],
            "unavailable"
        );
        assert_eq!(
            body["provider"]["capabilities"]["research"]["availability"],
            "unavailable"
        );
        let available_ids = body["provider"]["available_model_ids"].as_array().unwrap();
        assert_eq!(available_ids.len(), 1);
        assert_eq!(available_ids[0], "tablinq-instant");

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_non_streaming_returns_openai_shape() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
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
        assert!(body["tablinq_conversation_id"].as_str().is_some());
        assert_eq!(body["tablinq_session_id"], "browser-primary");
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
    fn chat_completions_support_fresh_and_existing_conversations() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(fake_url);
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let first_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "start the first conversation" }]
        }))
        .unwrap();
        let first = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
            ],
            &first_body,
            20_000,
        )
        .unwrap();
        assert_eq!(first.status_code, 200);
        let first_json: Value = serde_json::from_slice(&first.body).unwrap();
        let first_conversation_id = first_json["tablinq_conversation_id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(first_json["tablinq_session_id"], "browser-primary");

        let continue_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "tablinq_conversation_id": first_conversation_id,
            "messages": [{ "role": "user", "content": "continue the existing conversation" }]
        }))
        .unwrap();
        let continued = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
            ],
            &continue_body,
            20_000,
        )
        .unwrap();
        assert_eq!(continued.status_code, 200);
        let continued_json: Value = serde_json::from_slice(&continued.body).unwrap();
        assert_eq!(
            continued_json["tablinq_conversation_id"].as_str(),
            Some(first_conversation_id.as_str())
        );
        assert_eq!(continued_json["tablinq_session_id"], "browser-primary");

        let fresh_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "tablinq_new_conversation": true,
            "messages": [{ "role": "user", "content": "start a fresh conversation" }]
        }))
        .unwrap();
        let fresh = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
            ],
            &fresh_body,
            20_000,
        )
        .unwrap();
        assert_eq!(fresh.status_code, 200);
        let fresh_json: Value = serde_json::from_slice(&fresh.body).unwrap();
        assert_ne!(
            fresh_json["tablinq_conversation_id"].as_str(),
            Some(first_conversation_id.as_str())
        );
        assert_eq!(fresh_json["tablinq_session_id"], "browser-primary");

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_reject_unknown_conversation_ids() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(fake_url);
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "tablinq_conversation_id": "conv-missing",
            "messages": [{ "role": "user", "content": "fail with a missing conversation" }]
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
        assert_eq!(response.status_code, 404);
        let json: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(json["error"]["code"], "conversation_not_found");

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn sessions_endpoint_lists_recent_conversations() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(fake_url);
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let first_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "session listing baseline" }]
        }))
        .unwrap();
        let first = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
            ],
            &first_body,
            20_000,
        )
        .unwrap();
        let first_json: Value = serde_json::from_slice(&first.body).unwrap();
        let first_conversation_id = first_json["tablinq_conversation_id"]
            .as_str()
            .unwrap()
            .to_owned();

        let second_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "tablinq_new_conversation": true,
            "messages": [{ "role": "user", "content": "session listing fresh conversation" }]
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
        let second_json: Value = serde_json::from_slice(&second.body).unwrap();
        let second_conversation_id = second_json["tablinq_conversation_id"]
            .as_str()
            .unwrap()
            .to_owned();

        let sessions = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            V1_SESSIONS_PATH,
            &[("authorization", auth.as_str())],
            &[],
            10_000,
        )
        .unwrap();
        assert_eq!(sessions.status_code, 200);
        let sessions_json: Value = serde_json::from_slice(&sessions.body).unwrap();
        assert_eq!(sessions_json["object"], "tablinq.session_list");
        let session = &sessions_json["data"][0];
        assert_eq!(session["session_id"], "browser-primary");
        assert_eq!(session["latest_conversation_id"], second_conversation_id);
        let recent = session["recent_conversations"].as_array().unwrap();
        assert!(
            recent
                .iter()
                .any(|item| item["conversation_id"] == first_conversation_id)
        );
        assert!(
            recent
                .iter()
                .any(|item| item["conversation_id"] == second_conversation_id)
        );

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn request_summary_endpoint_persists_latency_and_unknown_usage_for_success() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(fake_url);
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "summarize latency" }]
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
                ("x-request-id", "req-metrics-success"),
            ],
            &body,
            20_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 200);

        let summary = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            "/v1/requests/req-metrics-success",
            &[("authorization", auth.as_str())],
            &[],
            10_000,
        )
        .unwrap();
        assert_eq!(summary.status_code, 200);
        let summary_body: Value = serde_json::from_slice(&summary.body).unwrap();
        assert_eq!(summary_body["data"]["request_id"], "req-metrics-success");
        assert_eq!(summary_body["data"]["status"], "completed");
        assert_eq!(summary_body["data"]["execution_kind"], "ask");
        assert_eq!(summary_body["data"]["cancelled"], false);
        assert!(summary_body["data"]["duration_ms"].as_u64().is_some());
        assert!(summary_body["data"]["queue_wait_ms"].as_u64().is_some());
        assert_eq!(summary_body["data"]["usage"]["status"], "unknown");
        assert!(
            summary_body["data"]["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value.as_str()
                    == Some("exact token usage is unavailable for the browser-backed provider"))
        );

        let usage = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            V1_USAGE_PATH,
            &[("authorization", auth.as_str())],
            &[],
            10_000,
        )
        .unwrap();
        assert_eq!(usage.status_code, 200);
        let usage_body: Value = serde_json::from_slice(&usage.body).unwrap();
        assert_eq!(usage_body["object"], "tablinq.request_summary_list");
        assert!(
            usage_body["data"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value["request_id"] == "req-metrics-success")
        );

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn request_summary_records_recovered_status_when_the_gateway_client_disconnects() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(fake_url);
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "stream": true,
            "messages": [{ "role": "user", "content": "recover after the gateway client disconnects" }]
        }))
        .unwrap();
        let mut stream = TcpStream::connect((&snapshot.host[..], snapshot.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        write!(
            stream,
            "POST {} HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: {}\r\nContent-Type: application/json\r\nX-Request-Id: req-client-drop\r\nContent-Length: {}\r\n\r\n",
            V1_CHAT_COMPLETIONS_PATH,
            snapshot.host,
            snapshot.port,
            auth,
            body.len()
        )
        .unwrap();
        stream.write_all(&body).unwrap();
        stream.flush().unwrap();
        let mut buffer = [0u8; 512];
        let _ = stream.read(&mut buffer).unwrap_or(0);
        drop(stream);

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let summary = http_request(
                &snapshot.host,
                snapshot.port,
                "GET",
                "/v1/requests/req-client-drop",
                &[("authorization", auth.as_str())],
                &[],
                10_000,
            )
            .unwrap();
            if summary.status_code == 200 {
                let summary_body: Value = serde_json::from_slice(&summary.body).unwrap();
                assert_eq!(summary_body["data"]["status"], "recovered");
                assert!(summary_body["data"]["interrupted_at"].as_str().is_some());
                assert!(summary_body["data"]["recovered_at"].as_str().is_some());
                assert!(
                    summary_body["data"]["interruption_reason"]
                        .as_str()
                        .unwrap()
                        .contains("client disconnected")
                );
                let config = AppConfig::load_or_create(Some(&state_root)).unwrap();
                let db = Database::open(&config.paths.db_path).unwrap();
                let request = db.request_summary("req-client-drop").unwrap().unwrap();
                assert_eq!(request.status, JobStatus::Recovered);
                assert!(request.recovered_at.is_some());
                break;
            }
            assert!(
                Instant::now() < deadline,
                "expected a recovered request summary after the client disconnected"
            );
            thread::sleep(Duration::from_millis(50));
        }

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn request_summary_endpoint_records_failed_requests_with_latency() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-failure"));
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let body = serde_json::to_vec(&json!({
            "model": "tablinq-research",
            "messages": [{ "role": "user", "content": "record a failed summary" }]
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
                ("x-request-id", "req-metrics-failure"),
            ],
            &body,
            20_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 502);

        let summary = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            "/v1/requests/req-metrics-failure",
            &[("authorization", auth.as_str())],
            &[],
            10_000,
        )
        .unwrap();
        assert_eq!(summary.status_code, 200);
        let summary_body: Value = serde_json::from_slice(&summary.body).unwrap();
        assert_eq!(summary_body["data"]["status"], "failed");
        assert_eq!(summary_body["data"]["execution_kind"], "research");
        assert_eq!(summary_body["data"]["cancelled"], false);
        assert!(summary_body["data"]["duration_ms"].as_u64().is_some());
        assert!(summary_body["data"]["end_time"].as_str().is_some());

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_streaming_emits_sse_chunks_and_done() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
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
        require_browser_integration_prereqs!();
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
        assert_eq!(body["tablinq_refusal_reason"], "capability_missing");
        assert!(body["tablinq_failure_bundle_id"].as_str().is_some());
        assert!(
            body["tablinq_guidance"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value
                    .as_str()
                    .is_some_and(|text| text.contains("refuses requests")))
        );

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn openai_auth_errors_include_refusal_guidance() {
        let err = anyhow::anyhow!("auth required: managed browser session is not authenticated");
        let response = map_openai_error("req-auth-expired", &err, Some("bundle-auth-1"));
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(response.status_code, 503);
        assert_eq!(body["error"]["code"], "auth_required");
        assert_eq!(body["tablinq_refusal_reason"], "auth_required");
        assert_eq!(body["tablinq_failure_bundle_id"], "bundle-auth-1");
        assert!(
            body["tablinq_guidance"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value.as_str().is_some_and(|text| text.contains("sign in")))
        );
    }

    #[test]
    fn files_upload_endpoint_stages_files_and_research_accepts_file_ids() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let upload = upload_file(
            &snapshot,
            &auth,
            "brief.txt",
            "text/plain",
            b"customer brief",
        );
        assert_eq!(upload.status_code, 201);
        let upload_body: Value = serde_json::from_slice(&upload.body).unwrap();
        let file_id = upload_body["data"]["id"].as_str().unwrap().to_owned();
        assert_eq!(upload_body["data"]["file_name"], "brief.txt");
        assert_eq!(upload_body["data"]["content_type"], "text/plain");
        assert_eq!(upload_body["data"]["size_bytes"], 14);
        assert!(upload_body["data"]["sha256"].as_str().is_some());
        assert!(
            upload_body["data"]["linked_request_ids"]
                .as_array()
                .unwrap()
                .is_empty()
        );

        let first_body = serde_json::to_vec(&json!({
            "model": "tablinq-research",
            "tablinq_file_ids": [file_id],
            "messages": [
                { "role": "user", "content": "summarize the upload" }
            ]
        }))
        .unwrap();
        let first = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
            ],
            &first_body,
            20_000,
        )
        .unwrap();
        assert_eq!(first.status_code, 200);
        let first_json: Value = serde_json::from_slice(&first.body).unwrap();
        assert!(
            first_json["choices"][0]["message"]["content"]
                .as_str()
                .unwrap()
                .contains("brief.txt")
        );
        let conversation_id = first_json["tablinq_conversation_id"]
            .as_str()
            .unwrap()
            .to_owned();

        let second_body = serde_json::to_vec(&json!({
            "model": "tablinq-research",
            "tablinq_file_ids": [file_id],
            "tablinq_conversation_id": conversation_id,
            "messages": [
                { "role": "user", "content": "reuse the same upload again" }
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
                ("x-request-id", "req-file-reuse-2"),
            ],
            &second_body,
            20_000,
        )
        .unwrap();
        assert_eq!(second.status_code, 200);

        let detail = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            &format!("{V1_FILES_PATH}/{file_id}"),
            &[("authorization", auth.as_str())],
            &[],
            20_000,
        )
        .unwrap();
        assert_eq!(detail.status_code, 200);
        let detail_json: Value = serde_json::from_slice(&detail.body).unwrap();
        assert_eq!(detail_json["data"]["id"], file_id);
        assert!(
            detail_json["data"]["linked_request_ids"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "req-file-reuse-2")
        );
        assert!(
            detail_json["data"]["linked_conversation_ids"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value.as_str().is_some())
        );

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn paperclip_like_http_flow_supports_files_and_artifacts() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let upload = upload_file(
            &snapshot,
            &auth,
            "paperclip-source.txt",
            "text/plain",
            b"paperclip source text",
        );
        assert_eq!(upload.status_code, 201);
        let upload_body: Value = serde_json::from_slice(&upload.body).unwrap();
        let file_id = upload_body["data"]["id"].as_str().unwrap().to_owned();

        let body = paperclip_request_body(
            "tablinq-research",
            "produce a summary report from the staged file",
            false,
            &[file_id.as_str()],
        );
        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
                ("x-client-id", "paperclip"),
            ],
            &body,
            20_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 200);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        let job_id = body["tablinq_job_id"].as_str().unwrap().to_owned();
        assert!(
            body["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value.as_str()
                    == Some("usage is unavailable for the browser-backed provider"))
        );

        let artifacts = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            V1_ARTIFACTS_PATH,
            &[("authorization", auth.as_str())],
            &[],
            20_000,
        )
        .unwrap();
        assert_eq!(artifacts.status_code, 200);
        let artifacts_body: Value = serde_json::from_slice(&artifacts.body).unwrap();
        let artifact = artifacts_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| {
                item["linked_job_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|value| value == &Value::String(job_id.clone()))
            })
            .unwrap();
        let content = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            artifact["content_url"].as_str().unwrap(),
            &[("authorization", auth.as_str())],
            &[],
            20_000,
        )
        .unwrap();
        assert_eq!(content.status_code, 200);
        assert!(
            String::from_utf8(content.body)
                .unwrap()
                .contains("paperclip-source.txt")
        );

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn provider_smoke_covers_chat_files_artifacts_and_cancel_flow() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let chat = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
            ],
            &serde_json::to_vec(&json!({
                "model": "tablinq-instant",
                "messages": [{ "role": "user", "content": "provider smoke ask" }]
            }))
            .unwrap(),
            20_000,
        )
        .unwrap();
        assert_eq!(chat.status_code, 200);

        let upload = upload_file(
            &snapshot,
            &auth,
            "smoke.txt",
            "text/plain",
            b"provider smoke file",
        );
        assert_eq!(upload.status_code, 201);
        let upload_body: Value = serde_json::from_slice(&upload.body).unwrap();
        let file_id = upload_body["data"]["id"].as_str().unwrap().to_owned();

        let artifact_response = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
            ],
            &paperclip_request_body(
                "tablinq-research",
                "create a smoke artifact",
                false,
                &[&file_id],
            ),
            20_000,
        )
        .unwrap();
        assert_eq!(artifact_response.status_code, 200);
        let artifact_response_body: Value =
            serde_json::from_slice(&artifact_response.body).unwrap();
        let request_id = artifact_response_body["request_id"]
            .as_str()
            .unwrap()
            .to_owned();

        let artifacts = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            V1_ARTIFACTS_PATH,
            &[("authorization", auth.as_str())],
            &[],
            20_000,
        )
        .unwrap();
        assert_eq!(artifacts.status_code, 200);
        let artifacts_body: Value = serde_json::from_slice(&artifacts.body).unwrap();
        let artifact_id = artifacts_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| {
                item["linked_request_ids"].as_array().is_some_and(|linked| {
                    linked
                        .iter()
                        .any(|value| value == &Value::String(request_id.clone()))
                })
            })
            .and_then(|item| item["id"].as_str())
            .unwrap()
            .to_owned();

        let artifact_detail = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            &format!("{V1_ARTIFACTS_PATH}/{artifact_id}"),
            &[("authorization", auth.as_str())],
            &[],
            20_000,
        )
        .unwrap();
        assert_eq!(artifact_detail.status_code, 200);

        let artifact_content = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            &format!("{V1_ARTIFACTS_PATH}/{artifact_id}/content"),
            &[("authorization", auth.as_str())],
            &[],
            20_000,
        )
        .unwrap();
        assert_eq!(artifact_content.status_code, 200);
        assert!(
            String::from_utf8(artifact_content.body)
                .unwrap()
                .contains("smoke.txt")
        );

        let cancel_body = serde_json::to_vec(&json!({
            "model": "tablinq-research",
            "messages": [{ "role": "user", "content": "cancel this smoke request" }]
        }))
        .unwrap();
        let host = snapshot.host.clone();
        let auth_for_thread = auth.clone();
        let running = thread::spawn(move || {
            http_request(
                &host,
                snapshot.port,
                "POST",
                V1_CHAT_COMPLETIONS_PATH,
                &[
                    ("authorization", auth_for_thread.as_str()),
                    ("content-type", "application/json"),
                    ("x-request-id", "req-provider-smoke-cancel"),
                ],
                &cancel_body,
                20_000,
            )
            .unwrap()
        });

        thread::sleep(Duration::from_millis(500));
        let cancel = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            "/v1/requests/req-provider-smoke-cancel/cancel",
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        assert_eq!(cancel.status_code, 200);
        let running = running.join().unwrap();
        assert_eq!(running.status_code, 409);

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn paperclip_like_requests_report_missing_staged_file_errors_cleanly() {
        let _guard = integration_lock();
        let (_dir, config) = test_config();
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let body = paperclip_request_body(
            "tablinq-instant",
            "read the staged file",
            false,
            &["file-missing"],
        );
        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
                ("x-client-id", "paperclip"),
            ],
            &body,
            20_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 400);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["error"]["code"], "file_not_found");

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn paperclip_like_queue_pressure_returns_retry_metadata() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
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

        let first_body = paperclip_request_body("tablinq-research", "hold the session", false, &[]);
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
                    ("x-client-id", "paperclip"),
                ],
                &first_body,
                20_000,
            )
        });

        thread::sleep(Duration::from_millis(150));

        let second_body =
            paperclip_request_body("tablinq-instant", "time out while queued", false, &[]);
        let second = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
                ("x-client-id", "paperclip"),
            ],
            &second_body,
            20_000,
        )
        .unwrap();
        assert_eq!(second.status_code, 429);
        let body: Value = serde_json::from_slice(&second.body).unwrap();
        assert_eq!(body["error"]["code"], "queue_timeout");
        assert_eq!(body["retry_after_ms"], 250);

        let first = first.join().unwrap().unwrap();
        assert_eq!(first.status_code, 200);

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn artifacts_endpoints_list_metadata_and_stream_content() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let upload = upload_file(
            &snapshot,
            &auth,
            "report.txt",
            "text/plain",
            b"artifact seed",
        );
        assert_eq!(upload.status_code, 201);
        let upload_body: Value = serde_json::from_slice(&upload.body).unwrap();
        let file_id = upload_body["data"]["id"].as_str().unwrap().to_owned();

        let body = serde_json::to_vec(&json!({
            "model": "tablinq-research",
            "tablinq_file_ids": [file_id],
            "messages": [
                { "role": "user", "content": "produce a downloadable report" }
            ]
        }))
        .unwrap();
        let completion = http_request(
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
        assert_eq!(completion.status_code, 200);
        let completion_body: Value = serde_json::from_slice(&completion.body).unwrap();
        let job_id = completion_body["tablinq_job_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let request_id = completion_body["request_id"].as_str().unwrap().to_owned();

        let artifacts = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            V1_ARTIFACTS_PATH,
            &[("authorization", auth.as_str())],
            &[],
            20_000,
        )
        .unwrap();
        assert_eq!(artifacts.status_code, 200);
        let artifacts_body: Value = serde_json::from_slice(&artifacts.body).unwrap();
        let artifact = artifacts_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| {
                item["linked_job_ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|value| value == &Value::String(job_id.clone()))
            })
            .unwrap();
        let artifact_id = artifact["id"].as_str().unwrap().to_owned();

        let detail = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            &format!("{V1_ARTIFACTS_PATH}/{artifact_id}"),
            &[("authorization", auth.as_str())],
            &[],
            20_000,
        )
        .unwrap();
        assert_eq!(detail.status_code, 200);
        let detail_body: Value = serde_json::from_slice(&detail.body).unwrap();
        assert_eq!(detail_body["data"]["id"], artifact_id);
        assert!(
            detail_body["data"]["linked_job_ids"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == &Value::String(job_id.clone()))
        );
        assert!(
            detail_body["data"]["linked_request_ids"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == &Value::String(request_id.clone()))
        );
        assert!(detail_body["data"]["created_at"].as_str().is_some());
        assert!(detail_body["data"]["sha256"].as_str().is_some());

        let content = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            &format!("{V1_ARTIFACTS_PATH}/{artifact_id}/content"),
            &[("authorization", auth.as_str())],
            &[],
            20_000,
        )
        .unwrap();
        assert_eq!(content.status_code, 200);
        let text = String::from_utf8(content.body).unwrap();
        assert!(text.contains("report.txt"));

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn files_endpoint_keeps_duplicate_original_names_as_separate_staged_ids() {
        let _guard = integration_lock();
        let (_dir, config) = test_config();
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let first = upload_file(&snapshot, &auth, "same.txt", "text/plain", b"one");
        let second = upload_file(&snapshot, &auth, "same.txt", "text/plain", b"two");
        assert_eq!(first.status_code, 201);
        assert_eq!(second.status_code, 201);

        let files = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            V1_FILES_PATH,
            &[("authorization", auth.as_str())],
            &[],
            20_000,
        )
        .unwrap();
        assert_eq!(files.status_code, 200);
        let body: Value = serde_json::from_slice(&files.body).unwrap();
        let matching = body["data"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["file_name"] == "same.txt")
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 2);
        assert_ne!(matching[0]["id"], matching[1]["id"]);
        assert_eq!(matching[0]["file_name"], "same.txt");
        assert_eq!(matching[1]["file_name"], "same.txt");
        assert_ne!(matching[0]["sha256"], matching[1]["sha256"]);

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn chat_completions_run_queued_requests_in_fifo_order() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
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
        require_browser_integration_prereqs!();
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

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = http_request(
                &snapshot.host,
                snapshot.port,
                "GET",
                ADMIN_STATUS_PATH,
                &[("authorization", auth.as_str())],
                &[],
                5_000,
            )
            .unwrap();
            assert_eq!(status.status_code, 200);
            let status_body: Value = serde_json::from_slice(&status.body).unwrap();
            if status_body["scheduler"]["queued"].as_u64().unwrap_or(0) >= 1 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "queued request never reached the scheduler"
            );
            thread::sleep(Duration::from_millis(25));
        }

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
        assert!(cancel_body["accepted"].as_bool().is_some());
        assert!(matches!(
            cancel_body["state"].as_str(),
            Some("queued" | "running")
        ));

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
        require_browser_integration_prereqs!();
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

        let summary = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            "/v1/requests/req-running-cancel",
            &[("authorization", auth.as_str())],
            &[],
            10_000,
        )
        .unwrap();
        assert_eq!(summary.status_code, 200);
        let summary_body: Value = serde_json::from_slice(&summary.body).unwrap();
        assert_eq!(summary_body["data"]["status"], "cancelled");
        assert_eq!(summary_body["data"]["cancelled"], true);
        assert!(summary_body["data"]["duration_ms"].as_u64().is_some());

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_cancel_running_streaming_ask_request() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(fake_url);
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "stream": true,
            "messages": [{ "role": "user", "content": "cancel a running ask stream" }]
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
                    ("x-request-id", "req-stream-cancel"),
                ],
                &body,
                20_000,
            )
            .unwrap()
        });

        thread::sleep(Duration::from_millis(250));

        let cancel = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            "/v1/requests/req-stream-cancel/cancel",
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        assert_eq!(cancel.status_code, 200);
        let cancel_body: Value = serde_json::from_slice(&cancel.body).unwrap();
        assert_eq!(cancel_body["accepted"], true);
        assert_eq!(cancel_body["state"], "running");
        assert!(
            cancel_body["message"]
                .as_str()
                .unwrap()
                .contains("cancellation requested")
        );

        let running = running.join().unwrap();
        assert_eq!(running.status_code, 200);
        let body = String::from_utf8(running.body).unwrap();
        assert!(body.contains("\"code\":\"cancelled\""));
        assert!(body.contains("[DONE]"));

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_cancel_during_reload_releases_the_queue_slot() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=reload"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.ask_timeout_ms = Some(10_000);
            gateway.queue_timeout_ms = Some(10_000);
        }
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let first_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "cancel me while the page reloads" }]
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
                    ("x-request-id", "req-reload-cancel"),
                ],
                &first_body,
                20_000,
            )
            .unwrap()
        });

        thread::sleep(Duration::from_millis(250));

        let cancel = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            "/v1/requests/req-reload-cancel/cancel",
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        assert_eq!(cancel.status_code, 200);

        let second_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "run after the cancelled reload request" }]
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
                ("x-request-id", "req-after-reload-cancel"),
            ],
            &second_body,
            20_000,
        )
        .unwrap();
        assert_eq!(second.status_code, 200);

        let first = first.join().unwrap();
        assert_eq!(first.status_code, 409);
        let first_body: Value = serde_json::from_slice(&first.body).unwrap();
        assert_eq!(first_body["error"]["code"], "cancelled");

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn chat_completions_return_queue_timeout_when_wait_expires() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
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
    fn gateway_rejects_missing_and_wrong_bearer_tokens_for_openai_routes() {
        let _guard = integration_lock();
        let (_dir, config) = test_config();
        let state_root = config.paths.state_root.clone();
        let (handle, snapshot) = spawn_gateway(config);
        let body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "reject unauthorized OpenAI-shaped requests" }]
        }))
        .unwrap();

        let missing = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[("content-type", "application/json")],
            &body,
            5_000,
        )
        .unwrap();
        assert_eq!(missing.status_code, 401);
        let missing_body: Value = serde_json::from_slice(&missing.body).unwrap();
        assert_eq!(missing_body["error"]["code"], "invalid_api_key");

        let wrong = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", "Bearer not-the-real-token"),
                ("content-type", "application/json"),
            ],
            &body,
            5_000,
        )
        .unwrap();
        assert_eq!(wrong.status_code, 401);
        let wrong_body: Value = serde_json::from_slice(&wrong.body).unwrap();
        assert_eq!(wrong_body["error"]["code"], "invalid_api_key");

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
    }

    #[test]
    fn chat_completions_return_auth_required_when_browser_session_is_signed_out() {
        let response = map_openai_error(
            "req-auth-expired",
            &anyhow::anyhow!("auth required: managed browser session is not authenticated"),
            Some("bundle-auth-1"),
        );
        assert_eq!(response.status_code, 503);
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["error"]["code"], "auth_required");
        assert_eq!(body["tablinq_refusal_reason"], "auth_required");
        assert_eq!(body["tablinq_failure_bundle_id"], "bundle-auth-1");
    }

    #[test]
    fn queue_reports_a_single_running_upstream_job_and_serializes_follow_up_work() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.max_concurrent_upstream_jobs = Some(1);
            gateway.queue_length = Some(4);
            gateway.queue_timeout_ms = Some(10_000);
        }
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let first_body = serde_json::to_vec(&json!({
            "model": "tablinq-research",
            "messages": [{ "role": "user", "content": "occupy the only upstream slot" }]
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
                    ("x-request-id", "req-single-slot-1"),
                ],
                &first_body,
                20_000,
            )
            .unwrap()
        });

        thread::sleep(Duration::from_millis(150));

        let second_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "wait behind the single slot" }]
        }))
        .unwrap();
        let host = snapshot.host.clone();
        let auth_for_thread = auth.clone();
        let second = thread::spawn(move || {
            http_request(
                &host,
                port,
                "POST",
                V1_CHAT_COMPLETIONS_PATH,
                &[
                    ("authorization", auth_for_thread.as_str()),
                    ("content-type", "application/json"),
                    ("x-request-id", "req-single-slot-2"),
                ],
                &second_body,
                20_000,
            )
            .unwrap()
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = http_request(
                &snapshot.host,
                snapshot.port,
                "GET",
                ADMIN_STATUS_PATH,
                &[("authorization", auth.as_str())],
                &[],
                5_000,
            )
            .unwrap();
            let status_body: Value = serde_json::from_slice(&status.body).unwrap();
            if status_body["scheduler"]["running"] == 1 && status_body["scheduler"]["queued"] == 1 {
                assert_eq!(status_body["scheduler"]["max_concurrent_upstream_jobs"], 1);
                break;
            }
            assert!(Instant::now() < deadline, "expected running=1 and queued=1");
            thread::sleep(Duration::from_millis(25));
        }

        let cancel = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            "/v1/requests/req-single-slot-1/cancel",
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        assert_eq!(cancel.status_code, 200);

        let first = first.join().unwrap();
        assert_eq!(first.status_code, 409);
        let second = second.join().unwrap();
        assert_eq!(second.status_code, 200);

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn queue_rejects_requests_when_per_client_limit_is_reached() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.per_client_limit = Some(1);
            gateway.queue_length = Some(4);
            gateway.queue_timeout_ms = Some(10_000);
        }
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let first_body = serde_json::to_vec(&json!({
            "model": "tablinq-research",
            "messages": [{ "role": "user", "content": "occupy the per-client slot" }]
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
                    ("x-client-id", "shared-client"),
                    ("x-request-id", "req-client-limit-1"),
                ],
                &first_body,
                20_000,
            )
            .unwrap()
        });

        thread::sleep(Duration::from_millis(150));

        let second_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "reject this second same-client request" }]
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
                ("x-client-id", "shared-client"),
            ],
            &second_body,
            20_000,
        )
        .unwrap();
        assert_eq!(second.status_code, 429);
        let second_body: Value = serde_json::from_slice(&second.body).unwrap();
        assert_eq!(second_body["error"]["code"], "client_limit_exceeded");
        assert!(second_body["retry_after_ms"].as_u64().is_some());

        let cancel = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            "/v1/requests/req-client-limit-1/cancel",
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        assert_eq!(cancel.status_code, 200);

        let first = first.join().unwrap();
        assert_eq!(first.status_code, 409);

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn queue_rejects_requests_when_the_queue_is_full() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.queue_length = Some(1);
            gateway.queue_timeout_ms = Some(10_000);
        }
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let first_body = serde_json::to_vec(&json!({
            "model": "tablinq-research",
            "messages": [{ "role": "user", "content": "occupy the slot" }]
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
                    ("x-request-id", "req-queue-full-1"),
                ],
                &first_body,
                20_000,
            )
            .unwrap()
        });

        thread::sleep(Duration::from_millis(150));

        let second_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "sit in the only queue slot" }]
        }))
        .unwrap();
        let host = snapshot.host.clone();
        let auth_for_thread = auth.clone();
        let second = thread::spawn(move || {
            http_request(
                &host,
                port,
                "POST",
                V1_CHAT_COMPLETIONS_PATH,
                &[
                    ("authorization", auth_for_thread.as_str()),
                    ("content-type", "application/json"),
                    ("x-request-id", "req-queue-full-2"),
                ],
                &second_body,
                20_000,
            )
            .unwrap()
        });

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let status = http_request(
                &snapshot.host,
                snapshot.port,
                "GET",
                ADMIN_STATUS_PATH,
                &[("authorization", auth.as_str())],
                &[],
                5_000,
            )
            .unwrap();
            let status_body: Value = serde_json::from_slice(&status.body).unwrap();
            if status_body["scheduler"]["queued"] == 1 {
                break;
            }
            assert!(Instant::now() < deadline, "expected a queued request");
            thread::sleep(Duration::from_millis(25));
        }

        let third_body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "overflow the queue" }]
        }))
        .unwrap();
        let third = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
                ("x-client-id", "other-client"),
            ],
            &third_body,
            20_000,
        )
        .unwrap();
        assert_eq!(third.status_code, 429);
        let third_body: Value = serde_json::from_slice(&third.body).unwrap();
        assert_eq!(third_body["error"]["code"], "queue_full");
        assert!(third_body["retry_after_ms"].as_u64().is_some());

        let cancel = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            "/v1/requests/req-queue-full-1/cancel",
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        assert_eq!(cancel.status_code, 200);

        let first = first.join().unwrap();
        assert_eq!(first.status_code, 409);
        let second = second.join().unwrap();
        assert_eq!(second.status_code, 200);

        stop_gateway_process(Some(&state_root)).unwrap();
        handle.join().unwrap().unwrap();
        let _ = fake_server.kill();
        let _ = fake_server.wait();
    }

    #[test]
    fn request_role_fields_flow_into_failure_bundles_and_request_summaries() {
        let _guard = integration_lock();
        require_browser_integration_prereqs!();
        let (mut fake_server, fake_url) = spawn_fake_chatgpt();
        let (_dir, mut config) = test_config();
        config.file.browser_target_url = Some(format!("{fake_url}/?scenario=timeout"));
        if let Some(gateway) = config.file.gateway.as_mut() {
            gateway.ask_timeout_ms = Some(1_000);
        }
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);

        let body = openclaude_request_body(
            "tablinq-instant",
            "fail and persist branch request role metadata",
            "branch",
            false,
            Some("req-parent-1"),
        );
        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-role-timeout"),
            ],
            &body,
            20_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 504);

        let summary = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            "/v1/requests/req-role-timeout",
            &[("authorization", auth.as_str())],
            &[],
            20_000,
        )
        .unwrap();
        assert_eq!(summary.status_code, 200);
        let summary_body: Value = serde_json::from_slice(&summary.body).unwrap();
        assert_eq!(summary_body["data"]["request_role"], "branch");
        assert_eq!(summary_body["data"]["parent_request_id"], "req-parent-1");

        let config = AppConfig::load_or_create(Some(&state_root)).unwrap();
        let db = Database::open(&config.paths.db_path).unwrap();
        let bundle = db.latest_failure_bundle().unwrap().expect("failure bundle");
        let gateway_request = bundle.gateway_request.expect("gateway request context");
        assert_eq!(gateway_request.request_id, "req-role-timeout");
        assert_eq!(gateway_request.request_role.as_deref(), Some("branch"));
        assert_eq!(
            gateway_request.parent_request_id.as_deref(),
            Some("req-parent-1")
        );

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
