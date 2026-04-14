use gptd::{AppConfig, CoreService};
use shared_protocol::{
    AskCommand, BrowserOpenRequest, CancelJobCommand, EventPayload, GatewayRequestContext, JobKind,
    JobStatus, Mode, PullArtifactCommand, ResearchCommand, StreamChunkKind,
};
use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
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

fn browser_integration_prereqs_available() -> bool {
    gptd::browser::browser_integration_prereqs_available()
}

#[test]
fn core_service_runs_ask_jobs_and_artifact_flow_without_cli() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed core_service test: browser adapter prerequisites are unavailable"
        );
        return;
    }
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();
    let upload_fixture = dir.path().join("brief.txt");
    fs::write(&upload_fixture, "brief body").unwrap();

    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let core = CoreService::new(config).unwrap();

    let open = core
        .browser_open(BrowserOpenRequest {
            target_url: Some(fake_url),
            browser_path: None,
            headless: Some(true),
        })
        .unwrap();
    assert!(open.running);
    assert!(open.authenticated);

    let capabilities = core.capabilities().unwrap();
    assert!(
        capabilities
            .report
            .as_ref()
            .expect("capability report")
            .matrix
            .submit_available
    );

    let mut streamed = String::new();
    let result = core
        .ask(
            AskCommand {
                prompt: "summarize uploads".to_owned(),
                conversation_id: None,
                new_conversation: false,
                mode: Mode::Normal,
                model_class: None,
                effort: None,
                files: vec![upload_fixture.display().to_string()],
            },
            |event| {
                if let EventPayload::StreamChunk(chunk) = event
                    && chunk.kind == StreamChunkKind::Delta
                {
                    streamed.push_str(&chunk.text);
                }
                Ok(())
            },
        )
        .unwrap();

    assert_eq!(result.output_text, streamed);
    assert_eq!(result.artifacts.len(), 1);

    let health = core.health_snapshot().unwrap();
    assert!(health.browser_connected);
    assert!(health.authenticated);

    let sessions = core.list_sessions().unwrap();
    assert_eq!(sessions.sessions.len(), 1);

    let jobs = core.list_jobs().unwrap();
    assert_eq!(jobs.jobs.len(), 1);
    assert_eq!(jobs.jobs[0].status, JobStatus::Completed);

    let artifacts = core.list_artifacts().unwrap();
    assert_eq!(artifacts.artifacts.len(), 1);

    let pulled = core
        .pull_artifact(PullArtifactCommand {
            artifact_id: "last".to_owned(),
            destination_dir: dir.path().join("pulled").display().to_string(),
        })
        .unwrap();
    let pulled_body = fs::read_to_string(&pulled.destination_path).unwrap();
    assert!(pulled_body.contains("Uploads: brief.txt"));

    let _ = core.browser_close();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn core_service_runs_research_jobs_without_cli() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed core_service test: browser adapter prerequisites are unavailable"
        );
        return;
    }
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();
    let output_dir = dir.path().join("research-out");

    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let core = CoreService::new(config).unwrap();
    core.browser_open(BrowserOpenRequest {
        target_url: Some(format!("{fake_url}/?scenario=research-success")),
        browser_path: None,
        headless: Some(true),
    })
    .unwrap();

    let mut statuses = Vec::new();
    let result = core
        .research(
            ResearchCommand {
                query: "investigate launch blockers".to_owned(),
                conversation_id: None,
                new_conversation: false,
                files: vec![],
                output_dir: Some(output_dir.display().to_string()),
            },
            |event| {
                if let EventPayload::JobState(state) = event {
                    statuses.push(state.status);
                }
                Ok(())
            },
        )
        .unwrap();

    assert!(result.report_text.contains("Research report for"));
    assert!(result.output_path.is_some());
    assert!(statuses.contains(&JobStatus::WaitingForSources));
    assert!(statuses.contains(&JobStatus::Completed));

    let detail = core.job_detail(&result.job_id).unwrap();
    assert_eq!(detail.kind, JobKind::Research);
    assert_eq!(detail.status, JobStatus::Completed);
    assert!(detail.output_path.is_some());

    let _ = core.browser_close();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn prepared_jobs_persist_across_restart() {
    let _guard = integration_lock();
    let dir = tempdir().unwrap();

    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let core = CoreService::new(config.clone()).unwrap();
    let prepared = core
        .prepare_ask(AskCommand {
            prompt: "hold this in the queue".to_owned(),
            conversation_id: None,
            new_conversation: false,
            mode: Mode::Normal,
            model_class: None,
            effort: None,
            files: vec![],
        })
        .unwrap();
    drop(core);

    let restarted = CoreService::new(config).unwrap();
    let detail = restarted.job_detail(&prepared.job_id).unwrap();
    assert_eq!(detail.status, JobStatus::Queued);
    assert_eq!(detail.input_text.as_deref(), Some("hold this in the queue"));
}

#[test]
fn capture_failure_bundle_persists_gateway_request_context() {
    let _guard = integration_lock();
    let dir = tempdir().unwrap();

    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let core = CoreService::new(config).unwrap();
    let bundle = core
        .capture_failure_bundle(
            Some(Mode::Search),
            "ui drift: provider request could not recover",
            Some(GatewayRequestContext {
                request_id: "req-provider-1".to_owned(),
                request_path: "/v1/chat/completions".to_owned(),
                client_id: Some("openclaude".to_owned()),
                model: Some("tablinq-search".to_owned()),
                request_role: Some("primary".to_owned()),
                parent_request_id: None,
                job_id: Some("job-provider-1".to_owned()),
                streaming: true,
            }),
        )
        .unwrap();

    let stored = core
        .doctor_report()
        .unwrap()
        .latest_failure_bundle
        .expect("latest failure bundle");
    let gateway_request = stored.gateway_request.expect("gateway request context");
    assert_eq!(bundle.bundle_id, stored.bundle_id);
    assert_eq!(gateway_request.request_id, "req-provider-1");
    assert_eq!(gateway_request.request_path, "/v1/chat/completions");
    assert_eq!(gateway_request.client_id.as_deref(), Some("openclaude"));
    assert_eq!(gateway_request.model.as_deref(), Some("tablinq-search"));
    assert_eq!(gateway_request.request_role.as_deref(), Some("primary"));
    assert_eq!(gateway_request.job_id.as_deref(), Some("job-provider-1"));
    assert!(gateway_request.streaming);
}

#[test]
fn cancelled_jobs_report_cancelled_status_in_job_detail() {
    let _guard = integration_lock();
    let dir = tempdir().unwrap();

    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let core = CoreService::new(config).unwrap();
    let prepared = core
        .prepare_ask(AskCommand {
            prompt: "cancel before execution".to_owned(),
            conversation_id: None,
            new_conversation: false,
            mode: Mode::Normal,
            model_class: None,
            effort: None,
            files: vec![],
        })
        .unwrap();

    let ack = core
        .cancel_job(CancelJobCommand {
            job_id: prepared.job_id.clone(),
        })
        .unwrap();
    assert!(ack.accepted);

    let err = core
        .execute_prepared_ask(&prepared, |_event| Ok(()))
        .unwrap_err();
    assert!(format!("{err:#}").contains("cancelled before execution"));

    let detail = core.job_detail(&prepared.job_id).unwrap();
    assert_eq!(detail.status, JobStatus::Cancelled);
    assert_eq!(
        detail.message.as_deref(),
        Some("ask cancelled before execution")
    );
    assert!(!detail.cancel_requested);
}
