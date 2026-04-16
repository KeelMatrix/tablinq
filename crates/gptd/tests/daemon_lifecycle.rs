use chrono::{Duration, Utc};
use gptd::{AppConfig, Database, daemon_is_responsive, send_request, send_request_stream};
use interprocess::local_socket::{GenericNamespaced, Stream, ToNsName};
use interprocess::local_socket::traits::Stream as LocalSocketStreamTrait;
use shared_protocol::{
    AskCommand, BrowserOpenRequest, DaemonRequest, DaemonResponse, Effort, ErrorCode, EventPayload,
    HealthStatus, JobStatus, Mode, ModelClass, PullArtifactCommand, ResearchCommand,
    build_state_paths,
};
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock, mpsc};
use std::thread;
use std::time::{Duration as StdDuration, Instant};
use tempfile::tempdir;

fn integration_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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

fn spawn_daemon(state_root: &Path) -> TestChild {
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

fn stop_daemon(state_root: &Path, child: &mut TestChild) {
    let paths = build_state_paths(state_root);
    let _ = send_request(&paths.ipc_endpoint, &DaemonRequest::Shutdown);
    let _ = child.wait();
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
    let deadline = Instant::now() + StdDuration::from_secs(10);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return (child, url);
        }
        thread::sleep(StdDuration::from_millis(100));
    }

    panic!("fake-chatgpt did not become ready on port {port}");
}

fn browser_integration_prereqs_available() -> bool {
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

fn wait_for_job_id(state_root: &Path) -> String {
    let deadline = Instant::now() + StdDuration::from_secs(10);
    while Instant::now() < deadline {
        let response = send_request(
            &build_state_paths(state_root).ipc_endpoint,
            &DaemonRequest::JobsList,
        )
        .unwrap();
        if let DaemonResponse::Jobs(list) = response
            && let Some(job) = list.jobs.first()
        {
            return job.job_id.clone();
        }
        thread::sleep(StdDuration::from_millis(50));
    }

    panic!("job id was not created in time");
}

fn wait_for_job_status(state_root: &Path, job_id: &str, expected: JobStatus) {
    let deadline = Instant::now() + StdDuration::from_secs(10);
    while Instant::now() < deadline {
        let response = send_request(
            &build_state_paths(state_root).ipc_endpoint,
            &DaemonRequest::JobsShow(job_id.to_owned()),
        )
        .unwrap();
        if let DaemonResponse::Job(job) = response
            && job.status == expected
        {
            return;
        }
        thread::sleep(StdDuration::from_millis(50));
    }

    panic!("job {job_id} did not reach status {expected:?} in time");
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
    assert_eq!(db.migration_count().unwrap(), 7);

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
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed daemon test: browser adapter prerequisites are unavailable"
        );
        return;
    }
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
            assert!(snapshot.target_url.starts_with(&fake_url));
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
            assert!(report.browser_detection.browser_detected);
            assert!(report.browser_detection.profile_accessible);
            assert!(report.selector_pack.is_some());
            assert!(
                report
                    .capabilities
                    .as_ref()
                    .expect("capability report")
                    .matrix
                    .composer_ready
            );
            assert!(report.failure_bundle_count == 0);
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
fn daemon_doctor_reports_attach_first_recovery_guidance_when_browser_is_not_running() {
    let _guard = integration_lock();
    let dir = tempdir().unwrap();
    let mut daemon = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let doctor = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::Doctor,
    )
    .unwrap();

    match doctor {
        DaemonResponse::Doctor(report) => {
            assert!(!report.browser.running);
            assert!(
                report
                    .recovery_guidance
                    .iter()
                    .any(|item| item.contains("gpt browser attach"))
            );
            assert!(
                report
                    .recovery_guidance
                    .iter()
                    .any(|item| item.contains("gpt browser connection-info"))
            );
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut daemon);
}

#[test]
fn daemon_streams_ask_responses_and_persists_conversations() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed daemon test: browser adapter prerequisites are unavailable"
        );
        return;
    }
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
            new_conversation: false,
            mode: Mode::Normal,
            model_class: None,
            effort: None,
            files: vec![],
        }),
        |event| {
            match event {
                EventPayload::StreamChunk(chunk) if !chunk.text.is_empty() => {
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
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed daemon test: browser adapter prerequisites are unavailable"
        );
        return;
    }
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
            new_conversation: false,
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

    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let db = Database::open(&config.paths.db_path).unwrap();
    assert_eq!(db.count_failure_bundles().unwrap(), 1);
    let bundle = db.latest_failure_bundle().unwrap().expect("failure bundle");
    assert_eq!(bundle.error_code, ErrorCode::ActionTimeout);
    assert!(Path::new(bundle.dom_snapshot_path.as_deref().unwrap()).exists());
    assert!(Path::new(bundle.aria_snapshot_path.as_deref().unwrap()).exists());
    assert!(bundle.screenshot_path.is_some());

    let doctor = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::Doctor,
    )
    .unwrap();
    match doctor {
        DaemonResponse::Doctor(report) => {
            assert_eq!(report.failure_bundle_count, 1);
            assert!(report.latest_failure_bundle.is_some());
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut daemon);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn stuck_ask_does_not_block_later_status_or_doctor() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed daemon test: browser adapter prerequisites are unavailable"
        );
        return;
    }
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

    let endpoint = build_state_paths(dir.path()).ipc_endpoint;
    let ask_handle = thread::spawn({
        let endpoint = endpoint.clone();
        move || {
            send_request_stream(
                &endpoint,
                &DaemonRequest::Ask(AskCommand {
                    prompt: "keep hanging".to_owned(),
                    conversation_id: None,
                    new_conversation: false,
                    mode: Mode::Normal,
                    model_class: None,
                    effort: None,
                    files: vec![],
                }),
                |_event| Ok(()),
            )
            .unwrap()
        }
    });

    let job_id = wait_for_job_id(dir.path());
    wait_for_job_status(dir.path(), &job_id, JobStatus::Running);

    let status_started = Instant::now();
    let status = send_request(&endpoint, &DaemonRequest::Status).unwrap();
    assert!(status_started.elapsed() < StdDuration::from_secs(2));
    assert!(matches!(status, DaemonResponse::Status(_)));

    let doctor_started = Instant::now();
    let doctor = send_request(&endpoint, &DaemonRequest::Doctor).unwrap();
    assert!(doctor_started.elapsed() < StdDuration::from_secs(2));
    match doctor {
        DaemonResponse::Doctor(_) => {}
        DaemonResponse::Error(error) => {
            assert!(error.message.contains("browser busy"));
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let cancel = send_request(
        &endpoint,
        &DaemonRequest::JobsCancel(shared_protocol::CancelJobCommand {
            job_id: job_id.clone(),
        }),
    )
    .unwrap();
    assert!(matches!(cancel, DaemonResponse::Ack(_)));

    let response = ask_handle.join().unwrap();
    match response {
        DaemonResponse::Error(error) => {
            assert!(error.message.contains("cancelled"));
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut daemon);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn stuck_ask_does_not_block_later_browser_attach() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed daemon test: browser adapter prerequisites are unavailable"
        );
        return;
    }
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

    let endpoint = build_state_paths(dir.path()).ipc_endpoint;
    let ask_handle = thread::spawn({
        let endpoint = endpoint.clone();
        move || {
            send_request_stream(
                &endpoint,
                &DaemonRequest::Ask(AskCommand {
                    prompt: "hang before attach".to_owned(),
                    conversation_id: None,
                    new_conversation: false,
                    mode: Mode::Normal,
                    model_class: None,
                    effort: None,
                    files: vec![],
                }),
                |_event| Ok(()),
            )
            .unwrap()
        }
    });

    let job_id = wait_for_job_id(dir.path());
    wait_for_job_status(dir.path(), &job_id, JobStatus::Running);

    let attach_started = Instant::now();
    let attach = send_request(
        &endpoint,
        &DaemonRequest::BrowserOpen(BrowserOpenRequest {
            target_url: Some(fake_url.clone()),
            browser_path: None,
            headless: Some(true),
        }),
    )
    .unwrap();
    assert!(attach_started.elapsed() < StdDuration::from_secs(2));
    match attach {
        DaemonResponse::Browser(_) => {}
        DaemonResponse::Error(error) => {
            assert!(error.message.contains("browser busy"));
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let _ = send_request(
        &endpoint,
        &DaemonRequest::JobsCancel(shared_protocol::CancelJobCommand { job_id }),
    )
    .unwrap();
    let _ = ask_handle.join().unwrap();

    stop_daemon(dir.path(), &mut daemon);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn client_disconnect_during_ask_cancels_the_job() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed daemon test: browser adapter prerequisites are unavailable"
        );
        return;
    }
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

    let endpoint = build_state_paths(dir.path()).ipc_endpoint;
    let name = endpoint
        .clone()
        .to_ns_name::<GenericNamespaced>()
        .expect("convert endpoint to local socket name");
    let mut stream = Stream::connect(name).expect("connect to daemon");
    let raw = serde_json::to_string(&DaemonRequest::Ask(AskCommand {
        prompt: "disconnect mid-ask".to_owned(),
        conversation_id: None,
        new_conversation: false,
        mode: Mode::Normal,
        model_class: None,
        effort: None,
        files: vec![],
    }))
    .unwrap();
    stream.write_all(raw.as_bytes()).unwrap();
    stream.write_all(b"\n").unwrap();
    stream.flush().unwrap();
    drop(stream);

    let job_id = wait_for_job_id(dir.path());
    wait_for_job_status(dir.path(), &job_id, JobStatus::Cancelled);

    let status = send_request(&endpoint, &DaemonRequest::Status).unwrap();
    assert!(matches!(status, DaemonResponse::Status(_)));

    stop_daemon(dir.path(), &mut daemon);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn cancelled_ask_releases_browser_capacity() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed daemon test: browser adapter prerequisites are unavailable"
        );
        return;
    }
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

    let endpoint = build_state_paths(dir.path()).ipc_endpoint;
    let (job_tx, job_rx) = mpsc::channel();
    let ask_handle = thread::spawn({
        let endpoint = endpoint.clone();
        move || {
            send_request_stream(
                &endpoint,
                &DaemonRequest::Ask(AskCommand {
                    prompt: "cancel and recover".to_owned(),
                    conversation_id: None,
                    new_conversation: false,
                    mode: Mode::Normal,
                    model_class: None,
                    effort: None,
                    files: vec![],
                }),
                |event| {
                    if let EventPayload::JobState(state) = event
                        && state.status == JobStatus::Running
                    {
                        let _ = job_tx.send(state.job_id);
                    }
                    Ok(())
                },
            )
            .unwrap()
        }
    });

    let job_id = job_rx
        .recv_timeout(StdDuration::from_secs(10))
        .expect("running ask job id");
    let cancel = send_request(
        &endpoint,
        &DaemonRequest::JobsCancel(shared_protocol::CancelJobCommand {
            job_id: job_id.clone(),
        }),
    )
    .unwrap();
    assert!(matches!(cancel, DaemonResponse::Ack(_)));
    wait_for_job_status(dir.path(), &job_id, JobStatus::Cancelled);

    let response = ask_handle.join().unwrap();
    match response {
        DaemonResponse::Error(error) => assert!(error.message.contains("cancelled")),
        other => panic!("unexpected response: {other:?}"),
    }

    let reopen = send_request(
        &endpoint,
        &DaemonRequest::BrowserOpen(BrowserOpenRequest {
            target_url: Some(fake_url.clone()),
            browser_path: None,
            headless: Some(true),
        }),
    )
    .unwrap();
    assert!(matches!(reopen, DaemonResponse::Browser(_)));

    stop_daemon(dir.path(), &mut daemon);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn daemon_handles_configured_asks_on_renamed_controls() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed daemon test: browser adapter prerequisites are unavailable"
        );
        return;
    }
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
            new_conversation: false,
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
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed daemon test: browser adapter prerequisites are unavailable"
        );
        return;
    }
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
            new_conversation: false,
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
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed daemon test: browser adapter prerequisites are unavailable"
        );
        return;
    }
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
            new_conversation: false,
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
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed daemon test: browser adapter prerequisites are unavailable"
        );
        return;
    }
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
            new_conversation: false,
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
fn daemon_runs_research_jobs_and_persists_details_across_restart() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed daemon test: browser adapter prerequisites are unavailable"
        );
        return;
    }
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();
    let upload_fixture = dir.path().join("brief.txt");
    let output_dir = dir.path().join("research-out");
    fs::write(&upload_fixture, "brief body").unwrap();

    let mut daemon = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let browser_open = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::BrowserOpen(BrowserOpenRequest {
            target_url: Some(format!("{fake_url}/?scenario=research-success")),
            browser_path: None,
            headless: Some(true),
        }),
    )
    .unwrap();
    assert!(matches!(browser_open, DaemonResponse::Browser(_)));

    let mut statuses = Vec::new();
    let response = send_request_stream(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::Research(ResearchCommand {
            query: "investigate launch blockers".to_owned(),
            conversation_id: None,
            new_conversation: false,
            files: vec![upload_fixture.display().to_string()],
            output_dir: Some(output_dir.display().to_string()),
        }),
        |event| {
            if let EventPayload::JobState(state) = event {
                statuses.push(state.status);
            }
            Ok(())
        },
    )
    .unwrap();

    let job_id = match response {
        DaemonResponse::Research(result) => {
            assert!(
                result
                    .report_text
                    .contains("Research report for: investigate launch blockers")
            );
            assert_eq!(result.artifacts.len(), 1);
            assert!(result.output_path.is_some());
            result.job_id
        }
        other => panic!("unexpected response: {other:?}"),
    };

    assert!(statuses.contains(&JobStatus::WaitingForSources));
    assert!(statuses.contains(&JobStatus::Running));
    assert!(statuses.contains(&JobStatus::Completed));

    let detail = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::JobsShow(job_id.clone()),
    )
    .unwrap();
    match detail {
        DaemonResponse::Job(job) => {
            assert_eq!(job.job_id, job_id);
            assert_eq!(job.kind, shared_protocol::JobKind::Research);
            assert_eq!(job.status, JobStatus::Completed);
            assert!(job.output_text.unwrap().contains("Research report for"));
            assert!(job.output_path.is_some());
            assert_eq!(job.artifacts.len(), 1);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut daemon);
    let mut restarted = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let detail_after_restart = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::JobsShow(job_id),
    )
    .unwrap();
    match detail_after_restart {
        DaemonResponse::Job(job) => {
            assert_eq!(job.status, JobStatus::Completed);
            assert!(job.output_path.is_some());
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut restarted);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn daemon_cancels_running_research_jobs() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed daemon test: browser adapter prerequisites are unavailable"
        );
        return;
    }
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();
    let mut daemon = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let browser_open = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::BrowserOpen(BrowserOpenRequest {
            target_url: Some(format!("{fake_url}/?scenario=research-success")),
            browser_path: None,
            headless: Some(true),
        }),
    )
    .unwrap();
    assert!(matches!(browser_open, DaemonResponse::Browser(_)));

    let endpoint = build_state_paths(dir.path()).ipc_endpoint;
    let handle = thread::spawn(move || {
        send_request_stream(
            &endpoint,
            &DaemonRequest::Research(ResearchCommand {
                query: "cancel me".to_owned(),
                conversation_id: None,
                new_conversation: false,
                files: vec![],
                output_dir: None,
            }),
            |_event| Ok(()),
        )
        .unwrap()
    });

    let job_id = wait_for_job_id(dir.path());
    let cancel = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::JobsCancel(shared_protocol::CancelJobCommand {
            job_id: job_id.clone(),
        }),
    )
    .unwrap();
    assert!(matches!(cancel, DaemonResponse::Ack(_)));

    let response = handle.join().unwrap();
    match response {
        DaemonResponse::Error(error) => {
            assert_eq!(error.error_code, ErrorCode::ResearchJobFailed);
            assert!(error.message.contains("cancelled"));
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let detail = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::JobsShow(job_id),
    )
    .unwrap();
    match detail {
        DaemonResponse::Job(job) => {
            assert_eq!(job.status, JobStatus::Cancelled);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    stop_daemon(dir.path(), &mut daemon);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn daemon_reports_research_failures_with_research_error_code() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed daemon test: browser adapter prerequisites are unavailable"
        );
        return;
    }
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();
    let mut daemon = spawn_daemon(dir.path());
    wait_until_ready(dir.path());

    let browser_open = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::BrowserOpen(BrowserOpenRequest {
            target_url: Some(format!("{fake_url}/?scenario=research-failure")),
            browser_path: None,
            headless: Some(true),
        }),
    )
    .unwrap();
    assert!(matches!(browser_open, DaemonResponse::Browser(_)));

    let response = send_request_stream(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::Research(ResearchCommand {
            query: "retry this".to_owned(),
            conversation_id: None,
            new_conversation: false,
            files: vec![],
            output_dir: None,
        }),
        |_event| Ok(()),
    )
    .unwrap();

    match response {
        DaemonResponse::Error(error) => {
            assert_eq!(error.error_code, ErrorCode::ResearchJobFailed);
            assert!(error.message.contains("research job failed"));
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
