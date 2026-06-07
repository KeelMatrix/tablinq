use super::*;
use chrono::{DateTime, TimeZone, Utc};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

fn fixed_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 12, 3, 4, 5).unwrap()
}

#[test]
fn upload_limits_match_canonical_metadata() {
    let raw = include_str!("../../../shared/upload-limits.json");
    let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();

    assert_eq!(
        MAX_UPLOAD_FILE_BYTES,
        parsed["maxUploadFileBytes"].as_u64().unwrap()
    );
    assert_eq!(
        MAX_COMPOSER_UPLOAD_FILES,
        parsed["maxComposerUploadFiles"].as_u64().unwrap() as usize
    );
}

fn ask_command_fixture() -> AskCommand {
    AskCommand {
        prompt: "hello".to_owned(),
        conversation_id: Some("conv-1".to_owned()),
        new_conversation: false,
        mode: Mode::Search,
        model_class: Some(ModelClass::Thinking),
        effort: Some(Effort::Standard),
        files: vec!["fixtures/a.pdf".to_owned()],
        prompt_upload_threshold_chars: None,
        auto_recover_provider_generation_failure: true,
    }
}

fn capability_report_fixture() -> CapabilityReport {
    CapabilityReport {
        reported_at: fixed_time(),
        selector_pack_id: "chatgpt-default".to_owned(),
        selector_pack_version: "v1".to_owned(),
        matrix: CapabilityMatrix {
            composer_ready: true,
            submit_available: true,
            model_picker_visible: true,
            thinking_effort_visible: true,
            search_mode_visible: true,
            research_mode_visible: false,
            upload_control_visible: true,
            downloadable_artifacts_observable: false,
        },
        aliases: RuntimeAliases {
            modes: BTreeMap::from([
                ("normal".to_owned(), vec!["Chat".to_owned()]),
                ("search".to_owned(), vec!["Search".to_owned()]),
            ]),
            model_classes: BTreeMap::from([(
                "thinking".to_owned(),
                vec!["GPT-5.4 Thinking".to_owned()],
            )]),
            efforts: BTreeMap::from([("standard".to_owned(), vec!["Standard".to_owned()])]),
        },
        observed_ui: CapabilityObservedUi {
            composer_readiness: BinaryCapabilityObservation {
                status: CapabilityObservationStatus::Observed,
                observed: true,
            },
            submit_availability: BinaryCapabilityObservation {
                status: CapabilityObservationStatus::Observed,
                observed: true,
            },
            selected_model: ModelSelectionObservation {
                status: CapabilityObservationStatus::Observed,
                picker_visible: true,
                selected_model_class: Some("thinking".to_owned()),
                selected_label: Some("GPT-5.4 Thinking".to_owned()),
                labels: vec!["GPT-5.4 Thinking".to_owned()],
                unsupported_pro_selected: false,
                ambiguous_selection: false,
            },
            thinking_effort: ThinkingEffortObservation {
                status: CapabilityObservationStatus::Observed,
                controls_visible: true,
                selected_effort: Some("standard".to_owned()),
                labels: vec!["Standard".to_owned()],
            },
            search_mode: SearchModeObservation {
                status: CapabilityObservationStatus::Observed,
                visible: true,
                selected: true,
                labels: vec!["Search".to_owned()],
            },
            research_flow: ResearchFlowObservation {
                status: CapabilityObservationStatus::NotObserved,
                entrypoint_visible: false,
                on_research_url: false,
                labels: Vec::new(),
            },
            upload_availability: BinaryCapabilityObservation {
                status: CapabilityObservationStatus::Observed,
                observed: true,
            },
        },
    }
}

fn browser_status_snapshot_fixture() -> BrowserStatusSnapshot {
    BrowserStatusSnapshot {
        running: true,
        browser_kind: "msedge".to_owned(),
        profile_dir: "C:/tablinq/browser-profile".to_owned(),
        attach_profile_dir: Some("C:/tablinq/chrome-attach-profile".to_owned()),
        attach_profile_note: Some(CHROME_ATTACH_PROFILE_NOTE.to_owned()),
        downloads_dir: "C:/tablinq/downloads".to_owned(),
        target_url: "https://chatgpt.com/".to_owned(),
        current_url: Some("https://chatgpt.com/c/conv-1".to_owned()),
        page_title: Some("ChatGPT".to_owned()),
        page_ready: true,
        authenticated: true,
        reload_count: 2,
        screenshot_path: Some("C:/tablinq/crash-bundles/last.png".to_owned()),
        browser_path: Some("C:/Program Files/Microsoft/Edge/Application/msedge.exe".to_owned()),
        extension_connected: true,
        capability_report: None,
        attach_mode: BrowserConnectionMode::AttachFirst,
        browser_owner: BrowserOwner::Managed,
        cdp_attached: false,
        target_tab_found: true,
        target_tab_id: Some("tab-1".to_owned()),
        target_workspace_key: Some("https://chatgpt.com/".to_owned()),
        debug_endpoint: Some("http://127.0.0.1:9222".to_owned()),
        auth_source: AuthSource::Page,
        extension_required: true,
        last_capability_report_at: Some("2026-04-12T10:00:00Z".to_owned()),
        action: None,
        browser_process_closed: None,
        reason: None,
    }
}

#[test]
fn command_envelope_round_trips() {
    let envelope = CommandEnvelope {
        version: PROTOCOL_VERSION.to_owned(),
        request_id: "req-123".to_owned(),
        session_id: Some("session-abc".to_owned()),
        sent_at: fixed_time(),
        command: CommandPayload::Ask(ask_command_fixture()),
    };

    let json = serde_json::to_string_pretty(&envelope).unwrap();
    let restored: CommandEnvelope = serde_json::from_str(&json).unwrap();

    assert_eq!(restored, envelope);
    assert!(json.contains("\"type\": \"ask\""));
    envelope.validate().unwrap();
}

#[test]
fn reset_rate_limit_command_round_trips_all_buckets() {
    for (bucket, expected_json) in [
        (RateLimitBucket::Instant, r#"{"bucket":"instant"}"#),
        (RateLimitBucket::Thinking, r#"{"bucket":"thinking"}"#),
        (
            RateLimitBucket::NativeUpload,
            r#"{"bucket":"native_upload"}"#,
        ),
    ] {
        let command = ResetRateLimitCommand { bucket };
        let json = serde_json::to_string(&command).unwrap();
        assert_eq!(json, expected_json);
        assert_eq!(
            serde_json::from_str::<ResetRateLimitCommand>(expected_json).unwrap(),
            command
        );
    }
}

#[test]
fn reset_rate_limit_command_rejects_unknown_fields_and_buckets() {
    assert!(
        serde_json::from_str::<ResetRateLimitCommand>(r#"{"bucket":"instant","extra":true}"#)
            .is_err()
    );
    assert!(serde_json::from_str::<ResetRateLimitCommand>(r#"{"bucket":"unsupported"}"#).is_err());
}

#[test]
fn event_envelope_round_trips() {
    let envelope = EventEnvelope {
        version: PROTOCOL_VERSION.to_owned(),
        request_id: Some("req-123".to_owned()),
        emitted_at: fixed_time(),
        event: EventPayload::CapabilityReport(capability_report_fixture()),
    };

    let json = serde_json::to_string_pretty(&envelope).unwrap();
    let restored: EventEnvelope = serde_json::from_str(&json).unwrap();

    assert_eq!(restored, envelope);
    assert!(json.contains("\"type\": \"capability_report\""));
    envelope.validate().unwrap();
}

#[test]
fn rejects_blank_request_id() {
    let envelope = CommandEnvelope {
        version: PROTOCOL_VERSION.to_owned(),
        request_id: "   ".to_owned(),
        session_id: None,
        sent_at: fixed_time(),
        command: CommandPayload::HealthPing(HealthPingCommand::default()),
    };

    let err = envelope.validate().unwrap_err();
    assert_eq!(err, ValidationError::EmptyField("request_id"));
}

#[test]
fn ask_command_accepts_up_to_ten_files() {
    let command = AskCommand {
        prompt: "hello".to_owned(),
        conversation_id: None,
        new_conversation: false,
        mode: Mode::Normal,
        model_class: Some(ModelClass::Instant),
        effort: None,
        files: (0..10)
            .map(|index| format!("fixtures/input-{index}.txt"))
            .collect(),
        prompt_upload_threshold_chars: None,
        auto_recover_provider_generation_failure: true,
    };

    command.validate().unwrap();
}

#[test]
fn ask_command_rejects_research_mode() {
    let command = AskCommand {
        prompt: "hello".to_owned(),
        conversation_id: None,
        new_conversation: false,
        mode: Mode::Research,
        model_class: Some(ModelClass::Instant),
        effort: None,
        files: vec![],
        prompt_upload_threshold_chars: None,
        auto_recover_provider_generation_failure: true,
    };

    let err = command.validate().unwrap_err();
    assert_eq!(
        err,
        ValidationError::InvalidFieldCombination {
            field: "mode".to_owned(),
            message: "research must use the dedicated research command".to_owned(),
        }
    );
}

#[test]
fn research_command_rejects_more_than_ten_files() {
    let command = ResearchCommand {
        query: "investigate".to_owned(),
        conversation_id: None,
        new_conversation: false,
        files: (0..11)
            .map(|index| format!("fixtures/input-{index}.txt"))
            .collect(),
        output_dir: None,
    };

    let err = command.validate().unwrap_err();
    assert_eq!(
        err,
        ValidationError::TooManyItems {
            field: "files",
            max: 10,
            found: 11,
        }
    );
}

#[test]
fn ask_command_rejects_file_larger_than_fifty_mib() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("oversize.bin");
    fs::write(&path, vec![0_u8; (MAX_ATTACHMENT_FILE_BYTES as usize) + 1]).unwrap();
    let command = AskCommand {
        prompt: "hello".to_owned(),
        conversation_id: None,
        new_conversation: false,
        mode: Mode::Normal,
        model_class: Some(ModelClass::Instant),
        effort: None,
        files: vec![path.display().to_string()],
        prompt_upload_threshold_chars: None,
        auto_recover_provider_generation_failure: true,
    };

    let err = command.validate().unwrap_err();
    assert_eq!(
        err,
        ValidationError::FileTooLarge {
            field: "files",
            path: path.display().to_string(),
            max_bytes: MAX_ATTACHMENT_FILE_BYTES,
            found_bytes: MAX_ATTACHMENT_FILE_BYTES + 1,
        }
    );
}

#[test]
fn research_command_accepts_file_at_fifty_mib() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("limit.bin");
    fs::write(&path, vec![0_u8; MAX_ATTACHMENT_FILE_BYTES as usize]).unwrap();
    let command = ResearchCommand {
        query: "investigate".to_owned(),
        conversation_id: None,
        new_conversation: false,
        files: vec![path.display().to_string()],
        output_dir: None,
    };

    command.validate().unwrap();
}

#[test]
fn conversation_select_command_accepts_known_conversation_id() {
    ConversationSelectCommand {
        conversation_id: Some("conv-1".to_owned()),
        new_conversation: false,
    }
    .validate()
    .unwrap();
}

#[test]
fn conversation_select_command_accepts_new_conversation() {
    ConversationSelectCommand {
        conversation_id: None,
        new_conversation: true,
    }
    .validate()
    .unwrap();
}

#[test]
fn conversation_select_command_rejects_missing_target() {
    let err = ConversationSelectCommand {
        conversation_id: None,
        new_conversation: false,
    }
    .validate()
    .unwrap_err();

    assert_eq!(
        err,
        ValidationError::InvalidFieldCombination {
            field: "conversation_id".to_owned(),
            message: "conversation_id or new_conversation is required".to_owned(),
        }
    );
}

#[test]
fn conversation_select_command_rejects_conflicting_target_flags() {
    let err = ConversationSelectCommand {
        conversation_id: Some("conv-1".to_owned()),
        new_conversation: true,
    }
    .validate()
    .unwrap_err();

    assert_eq!(
        err,
        ValidationError::InvalidFieldCombination {
            field: "conversation_id".to_owned(),
            message: "conversation_id cannot be combined with new_conversation".to_owned(),
        }
    );
}

#[test]
fn selector_pack_validation_rejects_duplicate_strategy_ids() {
    let pack = SelectorPack {
        format_version: SELECTOR_PACK_FORMAT_VERSION,
        pack_id: "chatgpt-default".to_owned(),
        description: "baseline".to_owned(),
        updated_at: fixed_time(),
        capabilities: BTreeMap::from([(
            "composer_ready".to_owned(),
            vec![
                SelectorStrategy {
                    strategy_id: "composer-role".to_owned(),
                    kind: StrategyKind::AriaRole,
                    selector: "[role='textbox']".to_owned(),
                    score: 0.98,
                },
                SelectorStrategy {
                    strategy_id: "composer-role".to_owned(),
                    kind: StrategyKind::Css,
                    selector: "textarea".to_owned(),
                    score: 0.7,
                },
            ],
        )]),
    };

    let err = pack.validate().unwrap_err();
    assert_eq!(
        err,
        ValidationError::DuplicateStrategy {
            field: "composer_ready".to_owned(),
            strategy_id: "composer-role".to_owned(),
        }
    );
}

#[test]
fn browser_status_snapshot_round_trips_with_attach_fields() {
    let snapshot = browser_status_snapshot_fixture();

    let json = serde_json::to_string_pretty(&snapshot).unwrap();
    let restored: BrowserStatusSnapshot = serde_json::from_str(&json).unwrap();

    assert_eq!(restored, snapshot);
    assert!(json.contains("\"attach_mode\": \"attach_first\""));
    assert!(json.contains("\"attach_profile_dir\": \"C:/tablinq/chrome-attach-profile\""));
    assert!(json.contains("\"browser_owner\": \"managed\""));
    assert!(json.contains("\"auth_source\": \"page\""));
    assert!(json.contains("\"target_workspace_key\": \"https://chatgpt.com/\""));
}

#[test]
fn build_state_paths_includes_dedicated_chrome_attach_profile_dir() {
    let root = PathBuf::from("state-root");
    let paths = build_state_paths(&root);

    assert_eq!(
        paths.chrome_attach_profile_dir,
        root.join("chrome-attach-profile")
    );
    assert!(paths.chrome_attach_profile_dir.starts_with(&root));
    assert_ne!(paths.chrome_attach_profile_dir, paths.browser_profile_dir);
}
