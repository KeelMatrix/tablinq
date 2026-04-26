use super::process::TestChild;
use gptd::{daemon_is_responsive, send_request};
use shared_protocol::{DaemonRequest, build_state_paths};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration as StdDuration, Instant};

pub fn integration_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub fn spawn_daemon(state_root: &Path) -> TestChild {
    seed_test_config(state_root);
    let mut command = Command::new(env!("CARGO_BIN_EXE_gptd"));
    command
        .arg("serve")
        .arg("--state-root")
        .arg(state_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    TestChild::spawn(command)
}

fn seed_test_config(state_root: &Path) {
    let config_dir = state_root.join("config");
    fs::create_dir_all(&config_dir).expect("create test config dir");
    fs::write(
        config_dir.join("config.toml"),
        "version = 2\nbrowser_submit_jitter_min_ms = 0\nbrowser_submit_jitter_max_ms = 0\n",
    )
    .expect("write daemon test config");
}

pub fn wait_until_ready(state_root: &Path) {
    let paths = build_state_paths(state_root);
    let deadline = Instant::now() + StdDuration::from_secs(5);
    while Instant::now() < deadline {
        if daemon_is_responsive(&paths.ipc_endpoint) {
            return;
        }
        thread::sleep(StdDuration::from_millis(50));
    }
    panic!("daemon did not become ready");
}

pub fn stop_daemon(state_root: &Path, child: &mut TestChild) {
    let paths = build_state_paths(state_root);
    let _ = send_request(&paths.ipc_endpoint, &DaemonRequest::Shutdown);
    let _ = child.wait();
}

pub fn browser_integration_prereqs_available() -> bool {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let adapter_entry = repo_root
        .join("packages")
        .join("browser-adapter")
        .join("dist")
        .join("src")
        .join("service.js");
    let playwright_core = repo_root
        .join("node_modules")
        .join("playwright-core")
        .join("package.json");

    let node_available = Command::new("node")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false);

    node_available && adapter_entry.exists() && playwright_core.exists()
}
