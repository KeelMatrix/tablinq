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

    let config = core_test_config(dir.path());
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
                auto_recover_provider_generation_failure: true,
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
    assert_eq!(
        sessions.sessions[0].latest_conversation_id.as_deref(),
        Some(result.conversation_id.as_str())
    );

    let conversations = core.list_conversations().unwrap();
    assert_eq!(conversations.conversations.len(), 1);
    assert_eq!(
        conversations.conversations[0].conversation_id,
        result.conversation_id
    );
    assert_eq!(
        conversations.conversations[0].last_prompt.as_deref(),
        Some("summarize uploads")
    );
    assert_eq!(
        conversations.conversations[0].last_response.as_deref(),
        Some(result.output_text.as_str())
    );

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
