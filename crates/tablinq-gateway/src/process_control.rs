use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use gptd::AppConfig;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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
    pub extended_thinking_timeout_ms: u64,
    pub research_timeout_ms: u64,
    pub safe_mode_behavior: String,
    pub log_path: String,
}

#[derive(Debug)]
pub(crate) struct LockGuard {
    path: PathBuf,
}

pub(crate) struct GatewayRuntimeGuard {
    pub(crate) status_path: PathBuf,
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

    let deadline = Instant::now() + crate::STARTUP_TIMEOUT;
    let startup_probe_timeout_ms = config
        .gateway_request_timeout_ms()
        .min(crate::STARTUP_PROBE_TIMEOUT_MS);
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
            "extended_thinking_timeout_ms": snapshot.extended_thinking_timeout_ms,
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
        crate::HEALTH_PATH,
        None,
        config.gateway_request_timeout_ms(),
    )
    .ok();
    let admin = gateway_json_request(
        &snapshot,
        crate::ADMIN_STATUS_PATH,
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
    let response = crate::http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        crate::ADMIN_SHUTDOWN_PATH,
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

    let deadline = Instant::now() + crate::STARTUP_TIMEOUT;
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

pub(crate) fn write_status_snapshot(
    config: &AppConfig,
    snapshot: &GatewayStatusSnapshot,
) -> Result<()> {
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

pub(crate) fn gateway_status_path(config: &AppConfig) -> PathBuf {
    config.paths.state_root.join("gateway-status.json")
}

fn gateway_lock_path(config: &AppConfig) -> PathBuf {
    config.paths.state_root.join("gateway.lock")
}

pub(crate) fn gateway_log_path(config: &AppConfig) -> PathBuf {
    config.paths.logs_dir.join("gateway.log")
}

pub(crate) fn acquire_gateway_lock(config: &AppConfig) -> Result<LockGuard> {
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
    crate::http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        crate::HEALTH_PATH,
        &[],
        &[],
        timeout_ms,
    )
    .map(|response| response.status_code == 200)
    .unwrap_or(false)
}

fn gateway_is_ready(snapshot: &GatewayStatusSnapshot, timeout_ms: u64) -> bool {
    crate::http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        crate::READY_PATH,
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
    let response = crate::http_request(
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
