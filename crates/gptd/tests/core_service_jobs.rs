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
            prompt_upload_threshold_chars: None,
            auto_recover_provider_generation_failure: true,
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
            prompt_upload_threshold_chars: None,
            auto_recover_provider_generation_failure: true,
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
