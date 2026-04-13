use gptd::{AppConfig, CoreService};
use shared_protocol::{
    AskCommand, BrowserOpenRequest, EventPayload, JobKind, JobStatus, Mode, PullArtifactCommand,
    ResearchCommand, StreamChunkKind,
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

#[test]
fn core_service_runs_ask_jobs_and_artifact_flow_without_cli() {
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
                mode: Mode::Normal,
                model_class: None,
                effort: None,
                files: vec![upload_fixture.display().to_string()],
            },
            |event| {
                if let EventPayload::StreamChunk(chunk) = event {
                    if chunk.kind == StreamChunkKind::Delta {
                        streamed.push_str(&chunk.text);
                    }
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
