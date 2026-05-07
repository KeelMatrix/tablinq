use super::*;
use chrono::{TimeZone, Utc};
use serde_json::Value;
use shared_protocol::{
    ConversationRecoveryManifest, Effort, ErrorCode, JobKind, JobStatus, Mode, ModelClass,
    RecoveryManifestFile, RecoveryManifestFileKind, RecoveryManifestTurn, RecoveryPacket,
    RecoveryStrategy, RecoveryTrace, RequestExecutionKind, RequestSummary, RequestUsage,
};

fn fixed_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 20, 8, 0, 0).unwrap()
}

fn seed_conversation_fixture(config: &AppConfig) {
    let db = Database::open(&config.paths.db_path).unwrap();
    let now = fixed_time();
    let input_path = config.paths.state_root.join("brief.txt");
    let artifact_path = config.paths.state_root.join("artifact-1.txt");
    let manifest_artifact_path = config.paths.state_root.join("artifact-2.txt");
    std::fs::write(&input_path, "brief").unwrap();
    std::fs::write(&artifact_path, "artifact-1").unwrap();
    std::fs::write(&manifest_artifact_path, "artifact-2").unwrap();

    db.upsert_session(
        "session-1",
        "Browser session",
        Some("https://chatgpt.com/c/conv-inspect"),
        now + chrono::Duration::minutes(6),
    )
    .unwrap();

    let manifest = ConversationRecoveryManifest {
        conversation_id: "conv-inspect".to_owned(),
        archived_context: vec!["Earlier summarized context".to_owned()],
        recent_turns: vec![
            RecoveryManifestTurn {
                completed_at: now + chrono::Duration::minutes(1),
                request_text: "Exact request one".to_owned(),
                response_excerpt: "Exact response one".to_owned(),
                input_files: vec!["brief.txt".to_owned()],
                generated_artifacts: vec!["artifact-1.txt".to_owned()],
            },
            RecoveryManifestTurn {
                completed_at: now + chrono::Duration::minutes(3),
                request_text: "Exact request two".to_owned(),
                response_excerpt: "Recovered response excerpt".to_owned(),
                input_files: vec!["brief.txt".to_owned()],
                generated_artifacts: vec!["artifact-2.txt".to_owned()],
            },
            RecoveryManifestTurn {
                completed_at: now + chrono::Duration::minutes(5),
                request_text: "CLI request three".to_owned(),
                response_excerpt: "CLI response three".to_owned(),
                input_files: vec![],
                generated_artifacts: vec![],
            },
        ],
        tracked_input_files: vec![RecoveryManifestFile {
            kind: RecoveryManifestFileKind::InputAttachment,
            file_name: "brief.txt".to_owned(),
            local_path: input_path.display().to_string(),
            content_type: Some("text/plain".to_owned()),
            recorded_at: now + chrono::Duration::minutes(3),
        }],
        tracked_generated_artifacts: vec![RecoveryManifestFile {
            kind: RecoveryManifestFileKind::GeneratedArtifact,
            file_name: "artifact-2.txt".to_owned(),
            local_path: manifest_artifact_path.display().to_string(),
            content_type: Some("text/plain".to_owned()),
            recorded_at: now + chrono::Duration::minutes(3),
        }],
        updated_at: now + chrono::Duration::minutes(5),
    };

    db.upsert_conversation(
        "conv-inspect",
        "session-1",
        "Inspection conversation",
        Some("https://chatgpt.com/c/conv-inspect"),
        Some("CLI request three"),
        Some("CLI response three"),
        Some(&manifest),
        now + chrono::Duration::minutes(6),
    )
    .unwrap();

    db.insert_job(
        "job-1",
        JobStatus::Completed,
        Some("ask completed"),
        Some("conv-inspect"),
        now,
        now + chrono::Duration::minutes(1),
    )
    .unwrap();
    db.upsert_job_detail("job-1", JobKind::Ask, Some("Exact request one"), None)
        .unwrap();
    db.update_job_output(
        "job-1",
        Some("Exact response one"),
        Some("https://chatgpt.com/c/conv-inspect"),
        None,
    )
    .unwrap();
    db.insert_artifact(
        &shared_protocol::ArtifactMetadata {
            artifact_id: "artifact-1".to_owned(),
            conversation_id: Some("conv-inspect".to_owned()),
            message_id: Some("msg-1".to_owned()),
            file_name: "artifact-1.txt".to_owned(),
            content_type: "text/plain".to_owned(),
            size_bytes: 10,
            sha256: "sha-1".to_owned(),
            storage_path: artifact_path.display().to_string(),
        },
        now + chrono::Duration::minutes(1),
    )
    .unwrap();
    db.link_job_artifact("job-1", "artifact-1").unwrap();
    db.upsert_request_summary(&RequestSummary {
        request_id: "req-1".to_owned(),
        job_id: "job-1".to_owned(),
        model_id: Some("tablinq-instant".to_owned()),
        request_role: Some("primary".to_owned()),
        parent_request_id: None,
        execution_kind: RequestExecutionKind::Ask,
        status: JobStatus::Completed,
        cancelled: false,
        start_time: now,
        end_time: Some(now + chrono::Duration::minutes(1)),
        duration_ms: Some(60_000),
        queue_wait_ms: 0,
        interrupted_at: None,
        interruption_reason: None,
        recovered_at: None,
        recovery_reason: None,
        failed_at: None,
        failure_reason: None,
        recovery_trace: None,
        recovery_packet: None,
        usage: RequestUsage {
            status: "unknown".to_owned(),
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
        },
        warnings: vec![],
    })
    .unwrap();

    let recovery_trace = RecoveryTrace {
        recovery_available: true,
        recovery_attempted: true,
        recovery_strategy: Some(RecoveryStrategy::ReloadNewConversationReplayWithContext),
        failure_code: Some(ErrorCode::ProviderGenerationFailedNonretryable),
        failure_bundle_id: Some("bundle-1".to_owned()),
        prior_request_id: Some("req-1".to_owned()),
        replacement_request_id: Some("req-2-replay".to_owned()),
        prior_conversation_id: Some("conv-inspect".to_owned()),
        replacement_conversation_id: Some("conv-replay".to_owned()),
        recovery_attempt_count: 1,
    };
    let recovery_packet = RecoveryPacket {
        original_input: "Exact request two".to_owned(),
        mode: Mode::Normal,
        model_class: Some(ModelClass::Thinking),
        effort: Some(Effort::Extended),
        files: vec![input_path.display().to_string()],
        context_summary: Some("Context from recovery packet".to_owned()),
        context_manifest: Some(manifest.clone()),
        recovery_manifest_path: Some(
            config
                .paths
                .state_root
                .join("recovery-manifest.md")
                .display()
                .to_string(),
        ),
        replay_files: vec![input_path.display().to_string()],
        prior_request_id: Some("req-1".to_owned()),
        prior_conversation_id: Some("conv-inspect".to_owned()),
        failure_code: ErrorCode::ProviderGenerationFailedNonretryable,
        recovery_attempt_count: 1,
    };

    db.insert_job(
        "job-2",
        JobStatus::Recovered,
        Some("ask recovered"),
        Some("conv-inspect"),
        now + chrono::Duration::minutes(2),
        now + chrono::Duration::minutes(3),
    )
    .unwrap();
    db.upsert_job_detail("job-2", JobKind::Ask, Some("Exact request two"), None)
        .unwrap();
    db.update_job_recovery_metadata("job-2", Some(&recovery_trace), Some(&recovery_packet))
        .unwrap();
    db.upsert_request_summary(&RequestSummary {
        request_id: "req-2".to_owned(),
        job_id: "job-2".to_owned(),
        model_id: Some("tablinq-thinking-extended".to_owned()),
        request_role: Some("primary".to_owned()),
        parent_request_id: None,
        execution_kind: RequestExecutionKind::Ask,
        status: JobStatus::Recovered,
        cancelled: false,
        start_time: now + chrono::Duration::minutes(2),
        end_time: Some(now + chrono::Duration::minutes(3)),
        duration_ms: Some(60_000),
        queue_wait_ms: 100,
        interrupted_at: Some(now + chrono::Duration::minutes(2)),
        interruption_reason: Some("provider generation failed".to_owned()),
        recovered_at: Some(now + chrono::Duration::minutes(3)),
        recovery_reason: Some("fresh conversation replay succeeded".to_owned()),
        failed_at: None,
        failure_reason: None,
        recovery_trace: Some(recovery_trace),
        recovery_packet: Some(recovery_packet),
        usage: RequestUsage {
            status: "unknown".to_owned(),
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
        },
        warnings: vec!["replayed from preserved context".to_owned()],
    })
    .unwrap();

    db.insert_job(
        "job-3",
        JobStatus::Completed,
        Some("cli ask completed"),
        Some("conv-inspect"),
        now + chrono::Duration::minutes(4),
        now + chrono::Duration::minutes(5),
    )
    .unwrap();
    db.upsert_job_detail("job-3", JobKind::Ask, Some("CLI request three"), None)
        .unwrap();
    db.update_job_output(
        "job-3",
        Some("CLI response three"),
        Some("https://chatgpt.com/c/conv-inspect"),
        None,
    )
    .unwrap();
}

#[test]
fn conversation_detail_endpoint_returns_expected_shape() {
    let (_dir, config) = test_config();
    seed_conversation_fixture(&config);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/conversations/conv-inspect",
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["object"], "tablinq.conversation");
    assert_eq!(body["data"]["conversation_id"], "conv-inspect");
    assert_eq!(body["data"]["tablinq_conversation_id"], "conv-inspect");
    assert_eq!(body["data"]["session"]["session_id"], "session-1");
    assert_eq!(body["data"]["latest_request_id"], "req-2");
    assert_eq!(body["data"]["latest_job_id"], "job-3");
    assert_eq!(body["data"]["counts"]["request_count"], 2);
    assert_eq!(
        body["data"]["routes"]["messages"],
        "/v1/conversations/conv-inspect/messages"
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn conversation_list_endpoint_includes_route_hints() {
    let (_dir, config) = test_config();
    seed_conversation_fixture(&config);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/conversations",
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(
        body["data"][0]["routes"]["detail"],
        "/v1/conversations/conv-inspect"
    );
    assert_eq!(
        body["data"][0]["routes"]["messages"],
        "/v1/conversations/conv-inspect/messages"
    );
    assert_eq!(
        body["data"][0]["routes"]["snapshot"],
        "/v1/conversations/conv-inspect/snapshot"
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn conversation_messages_endpoint_returns_ordered_normalized_items() {
    let (_dir, config) = test_config();
    seed_conversation_fixture(&config);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/conversations/conv-inspect/messages",
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["object"], "tablinq.conversation_message_list");
    assert_eq!(body["pagination"]["total_items"], 7);
    assert_eq!(body["data"][0]["source_kind"], "summary_fallback");
    assert_eq!(body["data"][1]["source_kind"], "request_detail");
    assert_eq!(body["data"][2]["message_id"], "msg-1");
    assert_eq!(body["data"][3]["request_id"], "req-2");
    assert_eq!(body["data"][4]["source_kind"], "recovery_manifest");
    assert_eq!(body["data"][4]["exactness"], "excerpt");
    assert_eq!(body["data"][6]["content"], "CLI response three");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn conversation_messages_endpoint_paginates() {
    let (_dir, config) = test_config();
    seed_conversation_fixture(&config);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/conversations/conv-inspect/messages?limit=2&offset=1",
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["pagination"]["limit"], 2);
    assert_eq!(body["pagination"]["offset"], 1);
    assert_eq!(body["pagination"]["next_offset"], 3);
    assert_eq!(body["data"][0]["content"], "Exact request one");
    assert_eq!(body["data"][1]["content"], "Exact response one");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn conversation_snapshot_endpoint_returns_recovery_aware_payload() {
    let (_dir, config) = test_config();
    seed_conversation_fixture(&config);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/conversations/conv-inspect/snapshot",
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["object"], "tablinq.conversation_snapshot");
    assert_eq!(
        body["data"]["conversation"]["conversation_id"],
        "conv-inspect"
    );
    assert_eq!(body["data"]["latest_request_id"], "req-2");
    assert_eq!(
        body["data"]["latest_recovery_packet"]["original_input"],
        "Exact request two"
    );
    assert_eq!(body["data"]["replacement_request_id"], "req-2-replay");
    assert_eq!(body["data"]["replacement_conversation_id"], "conv-replay");
    assert_eq!(
        body["data"]["context_summary"],
        "Context from recovery packet"
    );
    assert_eq!(body["data"]["suitable_for_fresh_conversation_replay"], true);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn conversation_endpoints_return_conversation_not_found() {
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    for path in [
        "/v1/conversations/missing",
        "/v1/conversations/missing/messages",
        "/v1/conversations/missing/snapshot",
    ] {
        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            path,
            &[("authorization", auth.as_str())],
            &[],
            10_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 404, "{path}");
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert!(body["request_id"].as_str().is_some(), "{path}");
        assert!(body["error"].is_object(), "{path}");
        assert_eq!(body["error"]["type"], "not_found_error", "{path}");
        assert_eq!(body["error"]["code"], "conversation_not_found", "{path}");
        assert_eq!(body["error"]["param"], Value::Null, "{path}");
    }

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn session_conversations_endpoint_returns_scoped_list() {
    let (_dir, config) = test_config();
    seed_conversation_fixture(&config);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/sessions/session-1/conversations",
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["object"], "tablinq.session_conversation_list");
    assert_eq!(body["session"]["session_id"], "session-1");
    assert_eq!(body["data"][0]["conversation_id"], "conv-inspect");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}
