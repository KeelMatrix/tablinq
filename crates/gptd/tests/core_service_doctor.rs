#![allow(unused_imports)]
use gptd::browser::execution_error_metadata;
use gptd::{AppConfig, CoreService};
use shared_protocol::{
    AskCommand, BrowserConnectionMode, BrowserOpenRequest, CancelJobCommand,
    ConversationSelectCommand, ErrorCode, EventPayload, GatewayRequestContext, JobKind, JobStatus,
    Mode, PullArtifactCommand, ResearchCommand, StreamChunkKind,
};
mod support;
use std::fs;
use std::path::Path;
use support::{
    core_browser_integration_prereqs_available as browser_integration_prereqs_available,
    core_test_config, integration_lock, spawn_fake_chatgpt,
};
use tempfile::tempdir;

#[test]
fn doctor_report_guides_attach_first_bootstrap_when_browser_not_running() {
    let _guard = integration_lock();
    let dir = tempdir().unwrap();

    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let core = CoreService::new(config).unwrap();
    let report = core.doctor_report().unwrap();

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

#[test]
fn doctor_report_health_matches_live_browser_snapshot() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed core_service test: browser adapter prerequisites are unavailable"
        );
        return;
    }
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();

    let mut config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    config.file.browser_connection_mode = Some(BrowserConnectionMode::ManagedOnly);
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
    assert!(open.extension_connected);

    let report = core.doctor_report().unwrap();

    assert_eq!(report.health.browser_connected, report.browser.running);
    assert_eq!(
        report.health.extension_connected,
        report.browser.extension_connected
    );
    assert_eq!(report.health.authenticated, report.browser.authenticated);
    assert!(report.health.browser_connected);
    assert!(report.health.extension_connected);
    assert!(report.health.authenticated);

    let _ = core.browser_close();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn browser_reset_profile_is_rejected_in_attach_first_mode() {
    let _guard = integration_lock();
    let dir = tempdir().unwrap();

    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let core = CoreService::new(config).unwrap();
    let err = core.browser_reset_profile().unwrap_err();

    assert!(format!("{err:#}").contains("managed browser recovery"));
}

#[test]
fn browser_close_in_attach_first_reports_detach_without_edge_fallback() {
    let _guard = integration_lock();
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("config")).unwrap();
    fs::write(
        dir.path().join("config").join("config.toml"),
        "version = 2\nbrowser_channel = \"chrome\"\n",
    )
    .unwrap();

    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let core = CoreService::new(config).unwrap();
    let snapshot = core.browser_close().unwrap();
    let report = core.doctor_report().unwrap();

    assert_eq!(snapshot.action.as_deref(), Some("detached"));
    assert_eq!(
        snapshot.browser_owner,
        shared_protocol::BrowserOwner::External
    );
    assert_eq!(snapshot.browser_kind, "chrome");
    assert_eq!(snapshot.browser_process_closed, Some(false));
    assert!(!snapshot.cdp_attached);
    assert!(!snapshot.running);
    assert_eq!(report.browser.browser_kind, "chrome");
    assert!(
        report
            .recovery_guidance
            .iter()
            .any(|line| line.contains("gpt browser attach"))
    );
    assert!(
        !report
            .recovery_guidance
            .iter()
            .any(|line| line.contains("Chrome or Edge"))
    );
}
