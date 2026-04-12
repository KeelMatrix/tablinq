use chrono::{Duration, Utc};
use gptd::{AppConfig, Database, daemon_is_responsive, send_request};
use shared_protocol::{DaemonRequest, DaemonResponse, HealthStatus, JobStatus, build_state_paths};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration as StdDuration, Instant};
use tempfile::tempdir;

fn spawn_daemon(state_root: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_gptd"))
        .arg("serve")
        .arg("--state-root")
        .arg(state_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn gptd")
}

fn wait_until_ready(state_root: &Path) {
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

fn stop_daemon(state_root: &Path, child: &mut Child) {
    let paths = build_state_paths(state_root);
    let _ = send_request(&paths.ipc_endpoint, &DaemonRequest::Shutdown);
    let _ = child.wait();
}

#[test]
fn daemon_boots_and_responds_over_local_socket() {
    let dir = tempdir().unwrap();
    let mut child = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let response = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::Ping,
    )
    .unwrap();

    match response {
        DaemonResponse::Health(health) => {
            assert_eq!(health.status, HealthStatus::Ok);
            assert_eq!(health.browser_connected, false);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut child);
}

#[test]
fn daemon_creates_config_and_applies_db_migrations() {
    let dir = tempdir().unwrap();
    let mut child = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    assert!(config.paths.config_path.exists());
    assert!(config.paths.db_path.exists());

    let db = Database::open(&config.paths.db_path).unwrap();
    assert_eq!(db.migration_count().unwrap(), 2);

    stop_daemon(dir.path(), &mut child);
}

#[test]
fn jobs_persist_across_restarts() {
    let dir = tempdir().unwrap();
    let mut first = spawn_daemon(dir.path());
    wait_until_ready(dir.path());
    stop_daemon(dir.path(), &mut first);

    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let db = Database::open(&config.paths.db_path).unwrap();
    let now = Utc::now();
    db.insert_job(
        "job-persisted",
        JobStatus::Queued,
        Some("waiting"),
        None,
        now,
        now,
    )
    .unwrap();

    let mut second = spawn_daemon(dir.path());
    wait_until_ready(dir.path());
    let response = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::JobsList,
    )
    .unwrap();

    match response {
        DaemonResponse::Jobs(list) => {
            assert_eq!(list.jobs.len(), 1);
            assert_eq!(list.jobs[0].job_id, "job-persisted");
            assert_eq!(list.jobs[0].status, JobStatus::Queued);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut second);
}

#[test]
fn stale_lock_is_recovered_on_boot() {
    let dir = tempdir().unwrap();
    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let stale = serde_json::json!({
        "pid": 999999u32,
        "created_at": (Utc::now() - Duration::seconds(30)).to_rfc3339(),
        "ipc_endpoint": config.paths.ipc_endpoint,
    });
    std::fs::write(
        &config.paths.lock_path,
        serde_json::to_vec_pretty(&stale).unwrap(),
    )
    .unwrap();

    let mut child = spawn_daemon(dir.path());
    wait_until_ready(dir.path());
    assert!(daemon_is_responsive(&config.paths.ipc_endpoint));

    stop_daemon(dir.path(), &mut child);
}

#[cfg(windows)]
#[test]
fn windows_daemon_uses_namespaced_endpoint() {
    let dir = tempdir().unwrap();
    let paths = build_state_paths(dir.path());
    assert!(paths.ipc_endpoint.starts_with("tablinq-"));
    assert!(!paths.ipc_endpoint.contains('\\'));
}
