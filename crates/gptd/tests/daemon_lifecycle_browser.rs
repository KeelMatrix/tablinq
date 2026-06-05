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
                    prompt_upload_threshold_chars: None,
                    auto_recover_provider_generation_failure: true,
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
                    prompt_upload_threshold_chars: None,
                    auto_recover_provider_generation_failure: true,
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
