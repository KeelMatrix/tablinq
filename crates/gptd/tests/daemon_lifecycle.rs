use chrono::{Duration, Utc};
use gptd::{AppConfig, Database, daemon_is_responsive, send_request, send_request_stream};
use shared_protocol::{
    AskCommand, BrowserOpenRequest, DaemonRequest, DaemonResponse, Effort, ErrorCode, EventPayload,
    HealthStatus, JobStatus, Mode, ModelClass, PullArtifactCommand, build_state_paths,
};
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration as StdDuration, Instant};
use tempfile::tempdir;

fn integration_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("integration lock poisoned")
}

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
    let deadline = Instant::now() + StdDuration::from_secs(10);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return (child, url);
        }
        thread::sleep(StdDuration::from_millis(100));
    }

    panic!("fake-chatgpt did not become ready on port {port}");
}

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
            assert_eq!(health.status, HealthStatus::Ok);
            assert_eq!(health.browser_connected, false);
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
    assert_eq!(db.migration_count().unwrap(), 3);

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
fn daemon_reports_browser_health_from_adapter() {
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();
    let mut daemon = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let browser_open = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::BrowserOpen(BrowserOpenRequest {
            target_url: Some(fake_url.clone()),
            browser_path: None,
            headless: Some(true),
        }),
    )
    .unwrap();

    match browser_open {
        DaemonResponse::Browser(snapshot) => {
            assert!(snapshot.running);
            assert_eq!(snapshot.target_url, fake_url);
            assert!(snapshot.authenticated);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let doctor = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::Doctor,
    )
    .unwrap();

    match doctor {
        DaemonResponse::Doctor(report) => {
            assert!(report.browser.running);
            assert!(report.health.browser_connected);
            assert!(report.health.extension_connected);
            assert!(report.health.authenticated);
            assert!(report.capabilities.is_some());
            assert!(
                report
                    .capabilities
                    .as_ref()
                    .expect("capability report")
                    .matrix
                    .composer_ready
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let capabilities = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::Capabilities,
    )
    .unwrap();

    match capabilities {
        DaemonResponse::Capabilities(snapshot) => {
            let report = snapshot.report.expect("capability report");
            assert!(report.matrix.submit_available);
            assert!(report.matrix.model_picker_visible);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let browser_close = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::BrowserClose,
    )
    .unwrap();

    match browser_close {
        DaemonResponse::Browser(snapshot) => {
            assert!(!snapshot.running);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut daemon);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn daemon_streams_ask_responses_and_persists_conversations() {
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();
    let mut daemon = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let browser_open = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::BrowserOpen(BrowserOpenRequest {
            target_url: Some(fake_url.clone()),
            browser_path: None,
            headless: Some(true),
        }),
    )
    .unwrap();
    assert!(matches!(browser_open, DaemonResponse::Browser(_)));

    let mut streamed = String::new();
    let mut saw_running = false;
    let response = send_request_stream(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::Ask(AskCommand {
            prompt: "hello from daemon".to_owned(),
            conversation_id: None,
            mode: Mode::Normal,
            model_class: None,
            effort: None,
            files: vec![],
        }),
        |event| {
            match event {
                EventPayload::StreamChunk(chunk) if chunk.text.len() > 0 => {
                    streamed.push_str(&chunk.text);
                }
                EventPayload::JobState(state) if state.status == JobStatus::Running => {
                    saw_running = true;
                }
                _ => {}
            }
            Ok(())
        },
    )
    .unwrap();

    match response {
        DaemonResponse::Ask(result) => {
            assert_eq!(streamed, "Fake response for: hello from daemon");
            assert_eq!(result.output_text, streamed);
            assert!(result.conversation_id.starts_with("conv-"));
            assert!(result.message_id.starts_with("msg-"));
        }
        other => panic!("unexpected response: {other:?}"),
    }
    assert!(saw_running);

    let jobs = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::JobsList,
    )
    .unwrap();
    match jobs {
        DaemonResponse::Jobs(list) => {
            assert_eq!(list.jobs[0].status, JobStatus::Completed);
            assert!(list.jobs[0].conversation_id.is_some());
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut daemon);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn daemon_reports_timeout_errors_for_stalled_asks() {
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();
    let mut daemon = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let browser_open = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::BrowserOpen(BrowserOpenRequest {
            target_url: Some(format!("{fake_url}/?scenario=timeout")),
            browser_path: None,
            headless: Some(true),
        }),
    )
    .unwrap();
    assert!(matches!(browser_open, DaemonResponse::Browser(_)));

    let response = send_request_stream(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::Ask(AskCommand {
            prompt: "this will stall".to_owned(),
            conversation_id: None,
            mode: Mode::Normal,
            model_class: None,
            effort: None,
            files: vec![],
        }),
        |_event| Ok(()),
    )
    .unwrap();

    match response {
        DaemonResponse::Error(error) => {
            assert_eq!(error.error_code, ErrorCode::ActionTimeout);
            assert!(error.message.contains("timed out"));
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut daemon);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn daemon_handles_configured_asks_on_renamed_controls() {
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();
    let mut daemon = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let browser_open = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::BrowserOpen(BrowserOpenRequest {
            target_url: Some(format!("{fake_url}/?variant=aria")),
            browser_path: None,
            headless: Some(true),
        }),
    )
    .unwrap();
    assert!(matches!(browser_open, DaemonResponse::Browser(_)));

    let response = send_request_stream(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::Ask(AskCommand {
            prompt: "configured aria ask".to_owned(),
            conversation_id: None,
            mode: Mode::Research,
            model_class: Some(ModelClass::Thinking),
            effort: Some(Effort::Extended),
            files: vec![],
        }),
        |_event| Ok(()),
    )
    .unwrap();

    match response {
        DaemonResponse::Ask(result) => {
            assert_eq!(result.output_text, "Fake response for: configured aria ask");
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut daemon);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn daemon_reports_capability_missing_errors_for_unavailable_modes() {
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();
    let mut daemon = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let browser_open = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::BrowserOpen(BrowserOpenRequest {
            target_url: Some(format!("{fake_url}/?missing=search")),
            browser_path: None,
            headless: Some(true),
        }),
    )
    .unwrap();
    assert!(matches!(browser_open, DaemonResponse::Browser(_)));

    let response = send_request_stream(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::Ask(AskCommand {
            prompt: "search for release notes".to_owned(),
            conversation_id: None,
            mode: Mode::Search,
            model_class: None,
            effort: None,
            files: vec![],
        }),
        |_event| Ok(()),
    )
    .unwrap();

    match response {
        DaemonResponse::Error(error) => {
            assert_eq!(error.error_code, ErrorCode::CapabilityMissing);
            assert!(error.message.contains("search mode"));
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut daemon);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn daemon_persists_uploaded_artifacts_and_pulls_them_locally() {
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();
    let upload_fixture = dir.path().join("brief.txt");
    fs::write(&upload_fixture, "brief body").unwrap();

    let mut daemon = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let browser_open = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::BrowserOpen(BrowserOpenRequest {
            target_url: Some(fake_url.clone()),
            browser_path: None,
            headless: Some(true),
        }),
    )
    .unwrap();
    assert!(matches!(browser_open, DaemonResponse::Browser(_)));

    let response = send_request_stream(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::Ask(AskCommand {
            prompt: "summarize uploads".to_owned(),
            conversation_id: None,
            mode: Mode::Normal,
            model_class: None,
            effort: None,
            files: vec![upload_fixture.display().to_string()],
        }),
        |_event| Ok(()),
    )
    .unwrap();

    let artifact_id = match response {
        DaemonResponse::Ask(result) => {
            assert_eq!(result.artifacts.len(), 1);
            assert!(result.artifacts[0].file_name.contains("brief-summary"));
            result.artifacts[0].artifact_id.clone()
        }
        other => panic!("unexpected response: {other:?}"),
    };

    let artifacts = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::ArtifactsList,
    )
    .unwrap();
    match artifacts {
        DaemonResponse::Artifacts(list) => {
            assert_eq!(list.artifacts.len(), 1);
            assert_eq!(list.artifacts[0].artifact_id, artifact_id);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let pull_dir = dir.path().join("pulled");
    let pull = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::ArtifactPull(PullArtifactCommand {
            artifact_id: "last".to_owned(),
            destination_dir: pull_dir.display().to_string(),
        }),
    )
    .unwrap();
    match pull {
        DaemonResponse::ArtifactPull(result) => {
            let pulled = fs::read_to_string(result.destination_path).unwrap();
            assert!(pulled.contains("Uploads: brief.txt"));
            assert_eq!(result.artifact.artifact_id, artifact_id);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut daemon);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn daemon_reports_upload_errors_when_attachment_controls_are_missing() {
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();
    let upload_fixture = dir.path().join("brief.txt");
    fs::write(&upload_fixture, "brief body").unwrap();

    let mut daemon = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let browser_open = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::BrowserOpen(BrowserOpenRequest {
            target_url: Some(format!("{fake_url}/?missing=upload")),
            browser_path: None,
            headless: Some(true),
        }),
    )
    .unwrap();
    assert!(matches!(browser_open, DaemonResponse::Browser(_)));

    let response = send_request_stream(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::Ask(AskCommand {
            prompt: "summarize uploads".to_owned(),
            conversation_id: None,
            mode: Mode::Normal,
            model_class: None,
            effort: None,
            files: vec![upload_fixture.display().to_string()],
        }),
        |_event| Ok(()),
    )
    .unwrap();

    match response {
        DaemonResponse::Error(error) => {
            assert_eq!(error.error_code, ErrorCode::UploadFailed);
            assert!(error.message.contains("upload failed"));
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut daemon);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
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
