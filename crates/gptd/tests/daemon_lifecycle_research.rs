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
fn client_disconnect_during_research_cancels_the_job() {
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
    let name = endpoint
        .clone()
        .to_ns_name::<GenericNamespaced>()
        .expect("convert endpoint to local socket name");
    let mut stream = Stream::connect(name).expect("connect to daemon");
    let raw = serde_json::to_string(&DaemonRequest::Research(ResearchCommand {
        query: "disconnect mid-research".to_owned(),
        conversation_id: None,
        new_conversation: false,
        files: vec![],
        output_dir: None,
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
        &DaemonRequest::JobsShow(job_id.clone()),
    )
    .unwrap();
    match detail {
        DaemonResponse::Job(job) => {
            assert_eq!(job.status, JobStatus::Cancelled);
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let second_cancel = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::JobsCancel(shared_protocol::CancelJobCommand { job_id }),
    )
    .unwrap();
    match second_cancel {
        DaemonResponse::Ack(ack) => {
            assert!(!ack.accepted);
            assert!(ack.message.contains("already finished"));
        }
        other => panic!("unexpected response: {other:?}"),
    }

    let status = send_request(
        &build_state_paths(dir.path()).ipc_endpoint,
        &DaemonRequest::Status,
    )
    .unwrap();
    assert!(matches!(status, DaemonResponse::Status(_)));

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

    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let db = Database::open(&config.paths.db_path).unwrap();
    let jobs = db.list_jobs().unwrap();
    assert_eq!(jobs.jobs[0].status, JobStatus::Failed);
    assert!(
        jobs.jobs[0]
            .message
            .as_deref()
            .unwrap()
            .contains("research job failed")
    );

    stop_daemon(dir.path(), &mut daemon);
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}
