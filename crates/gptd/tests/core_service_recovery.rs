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
fn core_service_recovers_provider_generation_failure_with_fresh_conversation_replay() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed core_service test: browser adapter prerequisites are unavailable"
        );
        return;
    }
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();

    let config = core_test_config(dir.path());
    let core = CoreService::new(config).unwrap();
    core.browser_open(BrowserOpenRequest {
        target_url: Some(format!(
            "{fake_url}/?variant=live&scenario=provider-generation-failure-recover"
        )),
        browser_path: None,
        headless: Some(true),
    })
    .unwrap();

    let result = core
        .ask(
            AskCommand {
                prompt: "reason carefully about this tradeoff".to_owned(),
                conversation_id: None,
                new_conversation: false,
                mode: Mode::Normal,
                model_class: None,
                effort: None,
                files: vec![],
                prompt_upload_threshold_chars: None,
                auto_recover_provider_generation_failure: true,
            },
            |_event| Ok(()),
        )
        .unwrap();

    assert!(result.low_confidence);
    assert!(result.output_text.contains("Recovery note:"));
    assert!(result.output_text.contains(
        "Answer this request now, using the attachment for prior context when relevant:"
    ));
    assert!(!result.output_text.contains("Preserved context:"));
    assert!(
        result
            .output_text
            .contains("reason carefully about this tradeoff")
    );

    let trace = result.recovery_trace.as_ref().expect("recovery trace");
    assert!(trace.recovery_available);
    assert!(trace.recovery_attempted);
    assert_eq!(
        trace.failure_code,
        Some(ErrorCode::ProviderGenerationFailedNonretryable)
    );
    assert!(trace.prior_request_id.is_some());
    assert!(trace.replacement_request_id.is_some());
    assert!(trace.prior_conversation_id.is_some());
    assert!(trace.replacement_conversation_id.is_some());
    assert_ne!(
        trace.prior_conversation_id,
        trace.replacement_conversation_id
    );

    let packet = result.recovery_packet.as_ref().expect("recovery packet");
    assert_eq!(
        packet.original_input,
        "reason carefully about this tradeoff"
    );
    assert_eq!(
        packet.failure_code,
        ErrorCode::ProviderGenerationFailedNonretryable
    );
    assert_eq!(packet.recovery_attempt_count, 1);
    assert!(packet.context_summary.as_deref().is_some());

    let detail = core.job_detail(&result.job_id).unwrap();
    assert_eq!(detail.recovery_trace, result.recovery_trace);
    assert_eq!(detail.recovery_packet, result.recovery_packet);

    let _ = core.browser_close();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn conversation_new_leaves_existing_provider_thread_before_next_ask() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed core_service test: browser adapter prerequisites are unavailable"
        );
        return;
    }
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();

    let mut config = core_test_config(dir.path());
    config.file.browser_target_url = Some(format!("{fake_url}/?variant=live"));
    let core = CoreService::new(config).unwrap();
    core.browser_open(BrowserOpenRequest {
        target_url: None,
        browser_path: None,
        headless: Some(true),
    })
    .unwrap();

    let first = core
        .ask(
            AskCommand {
                prompt: "list popular programming languages".to_owned(),
                conversation_id: None,
                new_conversation: false,
                mode: Mode::Normal,
                model_class: None,
                effort: None,
                files: vec![],
                prompt_upload_threshold_chars: None,
                auto_recover_provider_generation_failure: true,
            },
            |_event| Ok(()),
        )
        .unwrap();

    let selection = core
        .select_conversation(ConversationSelectCommand {
            conversation_id: None,
            new_conversation: true,
        })
        .unwrap();

    assert_eq!(selection.conversation_id, None);
    assert_ne!(
        selection.current_url.as_deref(),
        Some(first.current_url.as_str())
    );
    assert!(
        !selection
            .current_url
            .as_deref()
            .unwrap_or_default()
            .contains(&format!("/c/{}", first.conversation_id))
    );

    let second = core
        .ask(
            AskCommand {
                prompt: "zig".to_owned(),
                conversation_id: None,
                new_conversation: false,
                mode: Mode::Normal,
                model_class: None,
                effort: None,
                files: vec![],
                prompt_upload_threshold_chars: None,
                auto_recover_provider_generation_failure: true,
            },
            |_event| Ok(()),
        )
        .unwrap();

    assert_ne!(second.conversation_id, first.conversation_id);

    let _ = core.browser_close();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn core_service_recovers_provider_generation_failure_with_durable_manifest_context() {
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
    core.browser_open(BrowserOpenRequest {
        target_url: Some(format!(
            "{fake_url}/?variant=live&scenario=provider-generation-failure-after-success"
        )),
        browser_path: None,
        headless: Some(true),
    })
    .unwrap();

    let first = core
        .ask(
            AskCommand {
                prompt: "summarize the attached brief".to_owned(),
                conversation_id: None,
                new_conversation: false,
                mode: Mode::Normal,
                model_class: None,
                effort: None,
                files: vec![upload_fixture.display().to_string()],
                prompt_upload_threshold_chars: None,
                auto_recover_provider_generation_failure: true,
            },
            |_event| Ok(()),
        )
        .unwrap();
    assert_eq!(first.artifacts.len(), 1);

    let second = core
        .ask(
            AskCommand {
                prompt: "reason carefully about this tradeoff".to_owned(),
                conversation_id: Some(first.conversation_id.clone()),
                new_conversation: false,
                mode: Mode::Normal,
                model_class: None,
                effort: None,
                files: vec![],
                prompt_upload_threshold_chars: None,
                auto_recover_provider_generation_failure: true,
            },
            |_event| Ok(()),
        )
        .unwrap();

    assert!(second.low_confidence);
    assert!(second.output_text.contains("Uploads:"));
    assert!(second.output_text.contains("brief.txt"));

    let packet = second.recovery_packet.as_ref().expect("recovery packet");
    let manifest = packet.context_manifest.as_ref().expect("context manifest");
    assert!(!manifest.recent_turns.is_empty());
    assert!(packet.recovery_manifest_path.as_deref().is_some());
    let manifest_path = packet
        .recovery_manifest_path
        .as_ref()
        .expect("manifest attachment path");
    assert!(Path::new(manifest_path).exists());
    let manifest_name = Path::new(manifest_path)
        .file_name()
        .and_then(|value| value.to_str())
        .expect("manifest attachment filename");
    assert!(second.output_text.contains(manifest_name));
    assert!(
        packet
            .replay_files
            .iter()
            .any(|file| file.ends_with("brief.txt"))
    );
    assert!(packet.replay_files.iter().any(|file| file.ends_with(".md")));

    let db = gptd::Database::open(&core.config().paths.db_path).unwrap();
    let stored_manifest = db
        .conversation_recovery_manifest(&second.conversation_id)
        .unwrap()
        .expect("stored manifest for recovered conversation");
    assert_eq!(stored_manifest.recent_turns.len(), 2);
    assert_eq!(
        stored_manifest.recent_turns[0].input_files,
        vec!["brief.txt".to_owned()]
    );

    let _ = core.browser_close();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn core_service_stops_after_second_provider_generation_failure() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed core_service test: browser adapter prerequisites are unavailable"
        );
        return;
    }
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();

    let config = core_test_config(dir.path());
    let core = CoreService::new(config).unwrap();
    core.browser_open(BrowserOpenRequest {
        target_url: Some(format!(
            "{fake_url}/?variant=live&scenario=provider-generation-failure-twice"
        )),
        browser_path: None,
        headless: Some(true),
    })
    .unwrap();

    let err = core
        .ask(
            AskCommand {
                prompt: "reason carefully about this tradeoff".to_owned(),
                conversation_id: None,
                new_conversation: false,
                mode: Mode::Normal,
                model_class: None,
                effort: None,
                files: vec![],
                prompt_upload_threshold_chars: None,
                auto_recover_provider_generation_failure: true,
            },
            |_event| Ok(()),
        )
        .unwrap_err();

    let metadata = execution_error_metadata(&err).expect("typed provider failure");
    assert_eq!(
        metadata.error_code,
        ErrorCode::ProviderGenerationFailedNonretryable
    );
    let trace = metadata.recovery_trace.as_ref().expect("recovery trace");
    assert!(trace.recovery_attempted);
    assert_eq!(trace.recovery_attempt_count, 1);
    assert!(trace.replacement_request_id.is_some());
    let packet = metadata.recovery_packet.as_ref().expect("recovery packet");
    assert_eq!(packet.recovery_attempt_count, 1);

    let _ = core.browser_close();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn core_service_can_disable_provider_generation_failure_auto_recovery() {
    if !browser_integration_prereqs_available() {
        eprintln!(
            "skipping browser-backed core_service test: browser adapter prerequisites are unavailable"
        );
        return;
    }
    let _guard = integration_lock();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let dir = tempdir().unwrap();

    let config = core_test_config(dir.path());
    let core = CoreService::new(config).unwrap();
    core.browser_open(BrowserOpenRequest {
        target_url: Some(format!(
            "{fake_url}/?variant=live&scenario=provider-generation-failure-recover"
        )),
        browser_path: None,
        headless: Some(true),
    })
    .unwrap();

    let err = core
        .ask(
            AskCommand {
                prompt: "reason carefully about this tradeoff".to_owned(),
                conversation_id: None,
                new_conversation: false,
                mode: Mode::Normal,
                model_class: None,
                effort: None,
                files: vec![],
                prompt_upload_threshold_chars: None,
                auto_recover_provider_generation_failure: false,
            },
            |_event| Ok(()),
        )
        .unwrap_err();

    let metadata = execution_error_metadata(&err).expect("typed provider failure");
    assert_eq!(
        metadata.error_code,
        ErrorCode::ProviderGenerationFailedNonretryable
    );
    let trace = metadata.recovery_trace.as_ref().expect("recovery trace");
    assert!(!trace.recovery_attempted);
    assert!(trace.replacement_request_id.is_none());
    let packet = metadata.recovery_packet.as_ref().expect("recovery packet");
    assert_eq!(
        packet.original_input,
        "reason carefully about this tradeoff"
    );

    let _ = core.browser_close();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}
