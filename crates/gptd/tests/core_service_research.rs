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
    let upload_fixture = dir.path().join("brief.txt");
    let second_upload_fixture = dir.path().join("notes.csv");
    let output_dir = dir.path().join("research-out");
    fs::write(&upload_fixture, "brief body").unwrap();
    fs::write(&second_upload_fixture, "col\nvalue").unwrap();

    let config = core_test_config(dir.path());
    let core = CoreService::new(config).unwrap();
    core.browser_open(BrowserOpenRequest {
        target_url: Some(format!(
            "{fake_url}/?variant=live&scenario=research-success"
        )),
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
                files: vec![
                    upload_fixture.display().to_string(),
                    second_upload_fixture.display().to_string(),
                ],
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
    assert_eq!(result.artifacts.len(), 1);
    let artifact_body = fs::read_to_string(&result.artifacts[0].storage_path).unwrap();
    assert!(artifact_body.contains("Uploads: brief.txt, notes.csv"));

    let detail = core.job_detail(&result.job_id).unwrap();
    assert_eq!(detail.kind, JobKind::Research);
    assert_eq!(detail.status, JobStatus::Completed);
    assert!(detail.output_path.is_some());
    assert_eq!(detail.artifacts.len(), 1);

    let sessions = core.list_sessions().unwrap();
    assert_eq!(
        sessions.sessions[0].latest_conversation_id.as_deref(),
        Some(result.conversation_id.as_str())
    );
    let conversations = core.list_conversations().unwrap();
    assert_eq!(
        conversations.conversations[0].conversation_id,
        result.conversation_id
    );
    assert_eq!(
        conversations.conversations[0].last_prompt.as_deref(),
        Some("investigate launch blockers")
    );
    assert!(
        conversations.conversations[0]
            .last_response
            .as_deref()
            .unwrap()
            .contains("Research report for")
    );

    let _ = core.browser_close();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn cancelled_research_jobs_report_cancelled_status_before_execution() {
    let _guard = integration_lock();
    let dir = tempdir().unwrap();

    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let core = CoreService::new(config).unwrap();
    let prepared = core
        .prepare_research(ResearchCommand {
            query: "cancel research before execution".to_owned(),
            conversation_id: None,
            new_conversation: false,
            files: vec![],
            output_dir: None,
        })
        .unwrap();

    let ack = core
        .cancel_job(CancelJobCommand {
            job_id: prepared.job_id.clone(),
        })
        .unwrap();
    assert!(ack.accepted);

    let mut cancelled_event = None;
    let err = core
        .execute_prepared_research(&prepared, |event| {
            if let EventPayload::JobState(state) = event {
                cancelled_event = Some(state);
            }
            Ok(())
        })
        .unwrap_err();
    assert!(format!("{err:#}").contains("research cancelled before execution"));

    let event = cancelled_event.expect("cancelled event");
    assert_eq!(event.status, JobStatus::Cancelled);
    assert_eq!(
        event.message.as_deref(),
        Some("research cancelled before execution")
    );

    let detail = core.job_detail(&prepared.job_id).unwrap();
    assert_eq!(detail.status, JobStatus::Cancelled);
    assert_eq!(
        detail.message.as_deref(),
        Some("research cancelled before execution")
    );
    assert!(!detail.cancel_requested);
}
