use anyhow::{Context, Result, bail};
use serde_json::Value;
use shared_protocol::{CancelJobCommand, DaemonRequest, build_state_paths, resolve_state_root};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::{Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant};

static EXECUTION_INTERRUPT_HANDLER: OnceLock<()> = OnceLock::new();
static EXECUTION_INTERRUPT_SENDER: OnceLock<Mutex<Option<mpsc::Sender<()>>>> = OnceLock::new();

pub(crate) fn send_daemon_request(
    state_root: Option<&Path>,
    request: DaemonRequest,
) -> Result<Value> {
    let config = gptd::AppConfig::load_or_create(state_root)?;
    let paths = build_state_paths(resolve_state_root(state_root));
    let timeout = Duration::from_millis(config.gateway_request_timeout_ms());
    let endpoint = paths.ipc_endpoint.clone();
    let response = run_with_timeout(
        timeout,
        format!("daemon request timed out after {} ms", timeout.as_millis()),
        move || {
            gptd::send_request(&endpoint, &request)
                .with_context(|| format!("connect to daemon at {endpoint}"))
        },
    )?;
    serde_json::to_value(response).context("serialize daemon response")
}

pub(crate) struct ActiveExecutionInterruptGuard;

impl Drop for ActiveExecutionInterruptGuard {
    fn drop(&mut self) {
        if let Ok(mut slot) = execution_interrupt_sender_slot().lock() {
            *slot = None;
        }
    }
}

fn execution_interrupt_sender_slot() -> &'static Mutex<Option<mpsc::Sender<()>>> {
    EXECUTION_INTERRUPT_SENDER.get_or_init(|| Mutex::new(None))
}

pub(crate) fn install_execution_interrupt_sender(
    sender: mpsc::Sender<()>,
) -> Result<ActiveExecutionInterruptGuard> {
    if EXECUTION_INTERRUPT_HANDLER.get().is_none() {
        ctrlc::set_handler(|| {
            let sender = execution_interrupt_sender_slot()
                .lock()
                .ok()
                .and_then(|slot| slot.clone());
            if let Some(sender) = sender {
                let _ = sender.send(());
            }
        })
        .context("install Ctrl+C handler")?;
        let _ = EXECUTION_INTERRUPT_HANDLER.set(());
    }
    let mut slot = execution_interrupt_sender_slot()
        .lock()
        .map_err(|_| anyhow::anyhow!("Ctrl+C handler state lock poisoned"))?;
    *slot = Some(sender);
    Ok(ActiveExecutionInterruptGuard)
}

pub(crate) fn request_job_cancellation(endpoint: &str, job_id: &str) -> Result<()> {
    let endpoint = endpoint.to_owned();
    let job_id = job_id.to_owned();
    let response = run_with_timeout(
        Duration::from_secs(5),
        format!("job cancellation timed out for {job_id}"),
        move || {
            gptd::send_request(
                &endpoint,
                &DaemonRequest::JobsCancel(CancelJobCommand { job_id }),
            )
            .context("request daemon job cancellation")
        },
    )?;
    match response {
        shared_protocol::DaemonResponse::Ack(_) => Ok(()),
        shared_protocol::DaemonResponse::Error(error) => bail!("{}", error.message),
        other => bail!("unexpected cancellation response: {other:?}"),
    }
}

pub(crate) fn run_with_timeout<T, F>(
    timeout: Duration,
    timeout_message: String,
    work: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.recv_timeout(timeout)
        .map_err(|_| anyhow::anyhow!(timeout_message))?
}

pub(crate) fn start_daemon(state_root: Option<&Path>) -> Result<String> {
    let resolved_state_root = resolve_state_root(state_root);
    let paths = build_state_paths(&resolved_state_root);

    if gptd::daemon_is_responsive(&paths.ipc_endpoint) {
        return Ok(serde_json::json!({
            "status": "already_running",
            "ipc_endpoint": paths.ipc_endpoint,
        })
        .to_string());
    }

    let daemon_bin = resolve_gptd_binary();
    let mut command = ProcessCommand::new(&daemon_bin);
    command
        .arg("serve")
        .arg("--state-root")
        .arg(&resolved_state_root)
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
        .with_context(|| format!("spawn daemon binary {}", daemon_bin.display()))?;

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if gptd::daemon_is_responsive(&paths.ipc_endpoint) {
            return Ok(serde_json::json!({
                "status": "started",
                "ipc_endpoint": paths.ipc_endpoint,
                "state_root": resolved_state_root.display().to_string(),
            })
            .to_string());
        }

        if let Some(status) = child.try_wait().context("poll daemon startup")? {
            bail!("daemon exited during startup with status {status}");
        }

        if Instant::now() >= deadline {
            bail!("daemon did not become ready within 5 seconds");
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn resolve_gptd_binary() -> PathBuf {
    if let Ok(value) = std::env::var("TABLINQ_GPTD_BIN") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    let current = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("gpt"));
    let mut sibling = current.clone();
    sibling.set_file_name(if cfg!(windows) { "gptd.exe" } else { "gptd" });
    if sibling.exists() {
        sibling
    } else {
        PathBuf::from(if cfg!(windows) { "gptd.exe" } else { "gptd" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_with_timeout_returns_completed_work() {
        let value = run_with_timeout(Duration::from_millis(100), "timed out".to_owned(), || {
            Ok::<_, anyhow::Error>(42)
        })
        .unwrap();

        assert_eq!(value, 42);
    }

    #[test]
    fn run_with_timeout_fails_for_stalled_work() {
        let err = run_with_timeout(Duration::from_millis(50), "timed out".to_owned(), || {
            std::thread::sleep(Duration::from_millis(200));
            Ok::<_, anyhow::Error>(())
        })
        .unwrap_err();

        assert!(format!("{err:#}").contains("timed out"));
    }
}
