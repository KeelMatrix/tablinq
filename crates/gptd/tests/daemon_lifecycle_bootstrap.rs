#![allow(unused_imports)]
use chrono::{Duration, Utc};
use gptd::{AppConfig, Database, daemon_is_responsive, send_request, send_request_stream};
use interprocess::local_socket::traits::Stream as LocalSocketStreamTrait;
use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};
use shared_protocol::{
    AskCommand, BrowserOpenRequest, DaemonRequest, DaemonResponse, Effort, ErrorCode, EventPayload,
    HealthStatus, JobStatus, Mode, ModelClass, PullArtifactCommand, ResearchCommand,
    build_state_paths,
};
mod support;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration as StdDuration, Instant};
use support::{
    browser_integration_prereqs_available, integration_lock, spawn_daemon, spawn_fake_chatgpt,
    stop_daemon, wait_for_job_id, wait_for_job_status, wait_until_ready,
};
use tempfile::tempdir;

#[test]
fn daemon_boots_and_responds_over_local_socket() {
    let _guard = integration_lock();
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
            assert_eq!(health.status, HealthStatus::Degraded);
            assert!(!health.browser_connected);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut child);
}

#[test]
fn daemon_creates_config_and_applies_db_migrations() {
    let _guard = integration_lock();
    let dir = tempdir().unwrap();
    let mut child = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    assert!(config.paths.config_path.exists());
    assert!(config.paths.db_path.exists());

    let db = Database::open(&config.paths.db_path).unwrap();
    assert_eq!(db.migration_count().unwrap(), 9);

    stop_daemon(dir.path(), &mut child);
}

#[test]
fn jobs_persist_across_restarts() {
    let _guard = integration_lock();
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
    let _guard = integration_lock();
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
    let _guard = integration_lock();
    let dir = tempdir().unwrap();
    let paths = build_state_paths(dir.path());
    assert!(paths.ipc_endpoint.starts_with("tablinq-"));
    assert!(!paths.ipc_endpoint.contains('\\'));
}
