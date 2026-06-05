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
            prompt_upload_threshold_chars: None,
            auto_recover_provider_generation_failure: true,
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
            prompt_upload_threshold_chars: None,
            auto_recover_provider_generation_failure: true,
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
    let jobs = db.list_jobs().unwrap();
    assert_eq!(jobs.jobs[0].status, JobStatus::Failed);
    assert!(
        jobs.jobs[0]
            .message
            .as_deref()
            .unwrap()
            .contains("timed out")
    );
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
        prompt_upload_threshold_chars: None,
        auto_recover_provider_generation_failure: true,
    }))
    .unwrap();
    stream.write_all(raw.as_bytes()).unwrap();
    stream.write_all(b"\n").unwrap();
    stream.flush().unwrap();
    drop(stream);

    let job_id = wait_for_job_id(dir.path());
    wait_for_job_status(dir.path(), &job_id, JobStatus::Cancelled);

    let detail = send_request(&endpoint, &DaemonRequest::JobsShow(job_id.clone())).unwrap();
    match detail {
        DaemonResponse::Job(job) => {
            assert_eq!(job.status, JobStatus::Cancelled);
            assert!(job.message.unwrap().contains("client disconnected"));
        }
        other => panic!("unexpected response: {other:?}"),
    }

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
                    prompt_upload_threshold_chars: None,
                    auto_recover_provider_generation_failure: true,
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
            mode: Mode::Normal,
            model_class: Some(ModelClass::Thinking),
            effort: Some(Effort::Extended),
            files: vec![],
            prompt_upload_threshold_chars: None,
            auto_recover_provider_generation_failure: true,
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
            prompt_upload_threshold_chars: None,
            auto_recover_provider_generation_failure: true,
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
            prompt_upload_threshold_chars: None,
            auto_recover_provider_generation_failure: true,
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
            prompt_upload_threshold_chars: None,
            auto_recover_provider_generation_failure: true,
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
