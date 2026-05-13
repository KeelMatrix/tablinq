use super::*;
use chrono::Duration;
use sha2::{Digest, Sha256};
use shared_protocol::{
    ArtifactMetadata, CapabilityMatrix, ConversationRecoveryManifest, FailureBundle, JobKind,
    RequestExecutionKind, RequestUsage, SessionList,
};
use std::fs;
use tempfile::{TempDir, tempdir};

fn test_db() -> (TempDir, Database) {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("tablinq.sqlite3")).unwrap();
    (dir, db)
}

fn unknown_usage() -> RequestUsage {
    RequestUsage {
        status: "unknown".to_owned(),
        prompt_tokens: None,
        completion_tokens: None,
        total_tokens: None,
    }
}

fn request_summary_fixture(
    request_id: impl Into<String>,
    job_id: impl Into<String>,
    timestamp: chrono::DateTime<Utc>,
) -> RequestSummary {
    RequestSummary {
        request_id: request_id.into(),
        job_id: job_id.into(),
        model_id: Some("tablinq-instant".to_owned()),
        request_role: Some("primary".to_owned()),
        parent_request_id: None,
        execution_kind: RequestExecutionKind::Ask,
        status: JobStatus::Completed,
        cancelled: false,
        start_time: timestamp,
        end_time: Some(timestamp),
        duration_ms: Some(1),
        queue_wait_ms: 0,
        interrupted_at: None,
        interruption_reason: None,
        recovered_at: None,
        recovery_reason: None,
        failed_at: None,
        failure_reason: None,
        recovery_trace: None,
        recovery_packet: None,
        usage: unknown_usage(),
        warnings: vec![
            "exact token usage is unavailable for the browser-backed provider".to_owned(),
        ],
    }
}

fn recovery_trace_fixture() -> RecoveryTrace {
    RecoveryTrace {
        recovery_available: true,
        recovery_attempted: true,
        recovery_strategy: Some(
            shared_protocol::RecoveryStrategy::ReloadNewConversationReplayWithContext,
        ),
        failure_code: Some(shared_protocol::ErrorCode::ProviderGenerationFailedNonretryable),
        failure_bundle_id: Some("bundle-1".to_owned()),
        prior_request_id: Some("job-1:provider-attempt-0".to_owned()),
        replacement_request_id: Some("job-1:provider-attempt-1".to_owned()),
        prior_conversation_id: Some("conv-old".to_owned()),
        replacement_conversation_id: Some("conv-new".to_owned()),
        recovery_attempt_count: 1,
    }
}

fn recovery_packet_fixture(now: chrono::DateTime<Utc>) -> RecoveryPacket {
    RecoveryPacket {
        original_input: "reason carefully about this tradeoff".to_owned(),
        mode: shared_protocol::Mode::Normal,
        model_class: Some(shared_protocol::ModelClass::Thinking),
        effort: Some(shared_protocol::Effort::Extended),
        files: vec!["C:\\input.txt".to_owned()],
        context_summary: Some("Prior conversation id: conv-old".to_owned()),
        context_manifest: Some(shared_protocol::ConversationRecoveryManifest {
            conversation_id: "conv-old".to_owned(),
            archived_context: vec!["Earlier request: explain the system".to_owned()],
            recent_turns: vec![shared_protocol::RecoveryManifestTurn {
                completed_at: now,
                request_text: "reason carefully about this tradeoff".to_owned(),
                response_excerpt: "The tradeoff depends on context.".to_owned(),
                input_files: vec!["input.txt".to_owned()],
                generated_artifacts: vec![],
            }],
            tracked_input_files: vec![shared_protocol::RecoveryManifestFile {
                kind: shared_protocol::RecoveryManifestFileKind::InputAttachment,
                file_name: "input.txt".to_owned(),
                local_path: "C:\\input.txt".to_owned(),
                content_type: None,
                recorded_at: now,
            }],
            tracked_generated_artifacts: vec![],
            updated_at: now,
        }),
        recovery_manifest_path: Some("C:\\state\\recovery-manifests\\job-1.md".to_owned()),
        replay_files: vec![
            "C:\\input.txt".to_owned(),
            "C:\\state\\recovery-manifests\\job-1.md".to_owned(),
        ],
        prior_request_id: Some("job-1:provider-attempt-0".to_owned()),
        prior_conversation_id: Some("conv-old".to_owned()),
        failure_code: shared_protocol::ErrorCode::ProviderGenerationFailedNonretryable,
        recovery_attempt_count: 1,
    }
}

fn header_value<'a>(headers: &'a [serde_json::Value], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|item| {
            item["name"]
                .as_str()
                .is_some_and(|value| value.eq_ignore_ascii_case(name))
        })
        .and_then(|item| item["value"].as_str())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn seed_session(db: &Database, now: chrono::DateTime<Utc>) {
    db.upsert_session(
        "session-1",
        "Browser session",
        Some("https://chatgpt.com/"),
        now,
    )
    .unwrap();
}

fn seed_conversation(
    db: &Database,
    conversation_id: &str,
    prompt: &str,
    response: &str,
    timestamp: chrono::DateTime<Utc>,
) {
    db.upsert_conversation(
        conversation_id,
        "session-1",
        &format!("Conversation {conversation_id}"),
        Some(&format!("https://chatgpt.com/c/{conversation_id}")),
        Some(prompt),
        Some(response),
        None,
        timestamp,
    )
    .unwrap();
}

#[test]
fn migrations_are_applied_on_open() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("tablinq.sqlite3")).unwrap();
    assert_eq!(
        db.migration_count().unwrap(),
        migrations::MIGRATIONS.len() as u32
    );
}

#[test]
fn gateway_http_requests_migration_creates_table_and_indexes() {
    let (_dir, db) = test_db();

    let table_count: u32 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'gateway_http_requests'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(table_count, 1);

    let mut rows = db
        .conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'gateway_http_requests'",
        )
        .unwrap();
    let indexes = rows
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();

    for index in [
        "gateway_http_requests_created_at_idx",
        "gateway_http_requests_request_id_idx",
        "gateway_http_requests_path_idx",
        "gateway_http_requests_status_idx",
    ] {
        assert!(indexes.iter().any(|name| name == index), "missing {index}");
    }
}

#[test]
fn gateway_http_header_redaction_is_case_insensitive() {
    let headers = gateway_http_headers_json([
        ("Authorization", "Bearer secret"),
        ("proxy-Authorization", "proxy secret"),
        ("COOKIE", "a=b"),
        ("set-cookie", "sid=secret"),
        ("X-API-Key", "api secret"),
        ("x-request-id", "req-visible"),
    ])
    .unwrap();
    let headers: serde_json::Value = serde_json::from_str(&headers).unwrap();
    let headers = headers.as_array().unwrap();

    for name in [
        "authorization",
        "proxy-authorization",
        "cookie",
        "set-cookie",
        "x-api-key",
    ] {
        let value = header_value(headers, name);
        assert_eq!(value, Some("[redacted]"));
    }
    assert_eq!(header_value(headers, "x-request-id"), Some("req-visible"));
}

#[test]
fn gateway_http_request_body_capture_truncates_and_hashes_full_bytes() {
    let (_dir, db) = test_db();
    let now = Utc::now();
    let body = vec![b'a'; GATEWAY_HTTP_BODY_CAPTURE_LIMIT + 17];
    let start = GatewayHttpRequestStart {
        request_id: Some("req-monitor-request-body".to_owned()),
        remote_addr: Some("127.0.0.1:1".to_owned()),
        method: "POST".to_owned(),
        path: "/v1/chat/completions".to_owned(),
        route: Some("ChatCompletions".to_owned()),
        request_headers_json: None,
        request_body: Some(capture_gateway_http_body(&body)),
        parse_error: None,
        started_at: now,
        created_at: now,
    };

    db.start_gateway_http_request(&start).unwrap();
    let record = db
        .latest_gateway_http_request("req-monitor-request-body")
        .unwrap()
        .unwrap();

    assert_eq!(
        record.request_body_text.unwrap().len(),
        GATEWAY_HTTP_BODY_CAPTURE_LIMIT
    );
    assert_eq!(record.request_body_sha256, Some(sha256_hex(&body)));
    assert!(record.request_body_truncated);
}

#[test]
fn gateway_http_response_body_capture_truncates_and_hashes_full_bytes() {
    let (_dir, db) = test_db();
    let now = Utc::now();
    let row_id = db
        .start_gateway_http_request(&GatewayHttpRequestStart {
            request_id: Some("req-monitor-response-body".to_owned()),
            remote_addr: Some("127.0.0.1:1".to_owned()),
            method: "GET".to_owned(),
            path: "/v1/models".to_owned(),
            route: Some("Models".to_owned()),
            request_headers_json: None,
            request_body: Some(capture_gateway_http_body(&[])),
            parse_error: None,
            started_at: now,
            created_at: now,
        })
        .unwrap();
    let body = vec![b'z'; GATEWAY_HTTP_BODY_CAPTURE_LIMIT + 29];

    db.finish_gateway_http_request(
        row_id,
        &GatewayHttpRequestFinish {
            response_status_code: Some(200),
            response_headers_json: None,
            response_body: Some(capture_gateway_http_body(&body)),
            response_capture_omitted_reason: None,
            finished_at: now,
            duration_ms: 5,
        },
    )
    .unwrap();
    let record = db
        .latest_gateway_http_request("req-monitor-response-body")
        .unwrap()
        .unwrap();

    assert_eq!(
        record.response_body_text.unwrap().len(),
        GATEWAY_HTTP_BODY_CAPTURE_LIMIT
    );
    assert_eq!(record.response_body_sha256, Some(sha256_hex(&body)));
    assert!(record.response_body_truncated);
}

#[test]
fn request_history_is_retained_with_a_fixed_bound() {
    let (_dir, db) = test_db();
    let start = Utc::now();

    for index in 0..(REQUEST_HISTORY_LIMIT as u64 + 5) {
        let timestamp = start + Duration::milliseconds(index as i64);
        db.upsert_request_summary(&request_summary_fixture(
            format!("req-{index}"),
            format!("job-{index}"),
            timestamp,
        ))
        .unwrap();
    }

    let history = db
        .list_request_summaries(REQUEST_HISTORY_LIMIT + 10)
        .unwrap();
    assert_eq!(history.requests.len(), REQUEST_HISTORY_LIMIT);
    assert_eq!(
        history.requests[0].request_id,
        format!("req-{}", REQUEST_HISTORY_LIMIT as u64 + 4)
    );
    assert_eq!(history.requests.last().unwrap().request_id, "req-5");
    assert!(db.request_summary("req-0").unwrap().is_none());
}

#[test]
fn recovery_metadata_roundtrips_through_request_history_and_job_detail() {
    let (_dir, db) = test_db();
    let now = Utc::now();
    let recovery_trace = recovery_trace_fixture();
    let recovery_packet = recovery_packet_fixture(now);

    db.upsert_request_summary(&RequestSummary {
        request_id: "req-1".to_owned(),
        job_id: "job-1".to_owned(),
        model_id: Some("tablinq-thinking-extended".to_owned()),
        request_role: Some("primary".to_owned()),
        parent_request_id: None,
        execution_kind: RequestExecutionKind::Ask,
        status: JobStatus::Recovered,
        cancelled: false,
        start_time: now,
        end_time: Some(now),
        duration_ms: Some(100),
        queue_wait_ms: 0,
        interrupted_at: None,
        interruption_reason: None,
        recovered_at: Some(now),
        recovery_reason: Some("fresh conversation replay succeeded".to_owned()),
        failed_at: Some(now),
        failure_reason: Some("provider generation failed".to_owned()),
        recovery_trace: Some(recovery_trace.clone()),
        recovery_packet: Some(recovery_packet.clone()),
        usage: unknown_usage(),
        warnings: vec![],
    })
    .unwrap();
    db.insert_job(
        "job-1",
        JobStatus::Recovered,
        Some("ask recovered"),
        Some("conv-new"),
        now,
        now,
    )
    .unwrap();
    db.upsert_job_detail("job-1", JobKind::Ask, Some("input"), None)
        .unwrap();
    db.update_job_recovery_metadata("job-1", Some(&recovery_trace), Some(&recovery_packet))
        .unwrap();

    let request = db.request_summary("req-1").unwrap().unwrap();
    assert_eq!(request.recovery_trace, Some(recovery_trace.clone()));
    assert_eq!(request.recovery_packet, Some(recovery_packet.clone()));

    let job = db.job_detail("job-1").unwrap().unwrap();
    assert_eq!(job.recovery_trace, Some(recovery_trace));
    assert_eq!(job.recovery_packet, Some(recovery_packet));
}

#[test]
fn job_recovery_metadata_can_be_cleared_after_completed_non_recovery() {
    let (_dir, db) = test_db();
    let now = Utc::now();
    let recovery_trace = recovery_trace_fixture();
    let recovery_packet = recovery_packet_fixture(now);

    db.insert_job(
        "job-clear-recovery",
        JobStatus::Recovered,
        Some("ask recovered"),
        Some("conv-new"),
        now,
        now,
    )
    .unwrap();
    db.upsert_job_detail("job-clear-recovery", JobKind::Ask, Some("input"), None)
        .unwrap();
    db.record_job_recovery(
        "job-clear-recovery",
        now,
        "recovered from an earlier lifecycle update",
        Some("https://chatgpt.com/c/conv-new"),
    )
    .unwrap();
    db.update_job_recovery_metadata(
        "job-clear-recovery",
        Some(&recovery_trace),
        Some(&recovery_packet),
    )
    .unwrap();

    db.clear_job_recovery("job-clear-recovery").unwrap();

    let job = db.job_detail("job-clear-recovery").unwrap().unwrap();
    assert!(job.recovered_at.is_none());
    assert!(job.recovery_reason.is_none());
    assert!(job.recovery_trace.is_none());
    assert!(job.recovery_packet.is_none());
}

#[test]
fn conversation_recovery_manifest_roundtrips() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("tablinq.sqlite3");
    let now = Utc::now();
    let db = Database::open(&path).unwrap();

    db.upsert_session(
        "session-1",
        "Browser session",
        Some("https://chatgpt.com/c/conv-1"),
        now,
    )
    .unwrap();

    let manifest = ConversationRecoveryManifest {
        conversation_id: "conv-1".to_owned(),
        archived_context: vec!["Older turn summary".to_owned()],
        recent_turns: vec![shared_protocol::RecoveryManifestTurn {
            completed_at: now,
            request_text: "Summarize the brief".to_owned(),
            response_excerpt: "The brief recommends a phased rollout.".to_owned(),
            input_files: vec!["brief.txt".to_owned()],
            generated_artifacts: vec!["rollout-summary.txt".to_owned()],
        }],
        tracked_input_files: vec![shared_protocol::RecoveryManifestFile {
            kind: shared_protocol::RecoveryManifestFileKind::InputAttachment,
            file_name: "brief.txt".to_owned(),
            local_path: dir.path().join("brief.txt").display().to_string(),
            content_type: None,
            recorded_at: now,
        }],
        tracked_generated_artifacts: vec![shared_protocol::RecoveryManifestFile {
            kind: shared_protocol::RecoveryManifestFileKind::GeneratedArtifact,
            file_name: "rollout-summary.txt".to_owned(),
            local_path: dir.path().join("rollout-summary.txt").display().to_string(),
            content_type: Some("text/plain".to_owned()),
            recorded_at: now,
        }],
        updated_at: now,
    };

    db.upsert_conversation(
        "conv-1",
        "session-1",
        "Conversation 1",
        Some("https://chatgpt.com/c/conv-1"),
        Some("prompt 1"),
        Some("response 1"),
        Some(&manifest),
        now,
    )
    .unwrap();

    let stored = db
        .conversation_recovery_manifest("conv-1")
        .unwrap()
        .expect("stored manifest");
    assert_eq!(stored, manifest);
}

#[test]
fn artifacts_are_pruned_with_a_fixed_bound() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("tablinq.sqlite3")).unwrap();
    let now = Utc::now();

    for index in 0..(ARTIFACT_HISTORY_LIMIT as u64 + 2) {
        let storage_path = dir.path().join(format!("artifact-{index}.txt"));
        fs::write(&storage_path, format!("artifact-{index}")).unwrap();
        db.insert_artifact(
            &ArtifactMetadata {
                artifact_id: format!("artifact-{index}"),
                conversation_id: Some("conv-1".to_owned()),
                message_id: Some(format!("msg-{index}")),
                file_name: format!("artifact-{index}.txt"),
                content_type: "text/plain".to_owned(),
                size_bytes: 10,
                sha256: format!("sha-{index}"),
                storage_path: storage_path.display().to_string(),
            },
            now + Duration::milliseconds(index as i64),
        )
        .unwrap();
    }

    let removed = db.prune_artifacts(ARTIFACT_HISTORY_LIMIT).unwrap();
    assert_eq!(removed, 2);
    assert_eq!(
        db.list_artifacts().unwrap().artifacts.len(),
        ARTIFACT_HISTORY_LIMIT
    );
    assert!(db.find_artifact("artifact-0").unwrap().is_none());
    assert!(!dir.path().join("artifact-0.txt").exists());
}

#[test]
fn conversation_summaries_are_listed_and_persisted() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("tablinq.sqlite3");
    let now = Utc::now();

    {
        let db = Database::open(&path).unwrap();
        db.upsert_session(
            "browser-primary",
            "Managed browser",
            Some("http://example.test"),
            now,
        )
        .unwrap();
        db.upsert_conversation(
            "conv-1",
            "browser-primary",
            "Managed conversation",
            Some("http://example.test/c/conv-1"),
            Some("first prompt"),
            Some("first response"),
            None,
            now,
        )
        .unwrap();
        db.upsert_conversation(
            "conv-2",
            "browser-primary",
            "Managed conversation",
            Some("http://example.test/c/conv-2"),
            Some("second prompt"),
            Some("second response"),
            None,
            now + Duration::seconds(1),
        )
        .unwrap();
    }

    let db = Database::open(&path).unwrap();
    let conversation = db.find_conversation("conv-1").unwrap().unwrap();
    assert_eq!(conversation.session_id, "browser-primary");
    assert_eq!(conversation.last_prompt.as_deref(), Some("first prompt"));

    let sessions = db.list_sessions().unwrap();
    assert_eq!(sessions.sessions.len(), 1);
    assert_eq!(
        sessions.sessions[0].latest_conversation_id.as_deref(),
        Some("conv-2")
    );
    assert_eq!(sessions.sessions[0].recent_conversations.len(), 2);
    assert_eq!(
        sessions.sessions[0].recent_conversations[0].conversation_id,
        "conv-2"
    );
}

#[test]
fn list_conversations_returns_newest_first() {
    let (_dir, db) = test_db();
    let now = Utc::now();

    seed_session(&db, now);
    seed_conversation(&db, "conv-1", "prompt 1", "response 1", now);
    seed_conversation(
        &db,
        "conv-2",
        "prompt 2",
        "response 2",
        now + Duration::seconds(1),
    );

    let conversations = db.list_conversations().unwrap();
    assert_eq!(conversations.conversations.len(), 2);
    assert_eq!(conversations.conversations[0].conversation_id, "conv-2");
    assert_eq!(conversations.conversations[1].conversation_id, "conv-1");
}

#[test]
fn session_queries_return_default_list_shape_when_empty() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("tablinq.sqlite3")).unwrap();

    assert_eq!(
        db.list_sessions().unwrap(),
        SessionList { sessions: vec![] }
    );
    assert!(db.find_session_info("missing").unwrap().is_none());
}

#[test]
fn clear_all_state_removes_persisted_rows_but_keeps_migrations() {
    let (dir, mut db) = test_db();
    let now = Utc::now();
    let artifact_path = dir.path().join("artifact-clear.txt");
    fs::write(&artifact_path, "artifact").unwrap();
    let manifest = ConversationRecoveryManifest {
        conversation_id: "conv-clear".to_owned(),
        archived_context: vec!["summary".to_owned()],
        recent_turns: vec![],
        tracked_input_files: vec![],
        tracked_generated_artifacts: vec![],
        updated_at: now,
    };
    let recovery_trace = recovery_trace_fixture();
    let recovery_packet = recovery_packet_fixture(now);

    db.upsert_session(
        "session-clear",
        "Session",
        Some("https://chatgpt.com/"),
        now,
    )
    .unwrap();
    db.upsert_conversation(
        "conv-clear",
        "session-clear",
        "Conversation",
        Some("https://chatgpt.com/c/conv-clear"),
        Some("prompt"),
        Some("response"),
        Some(&manifest),
        now,
    )
    .unwrap();
    db.insert_job(
        "job-clear",
        JobStatus::Recovered,
        Some("recovered"),
        Some("conv-clear"),
        now,
        now,
    )
    .unwrap();
    db.upsert_job_detail("job-clear", JobKind::Ask, Some("prompt"), None)
        .unwrap();
    db.update_job_recovery_metadata("job-clear", Some(&recovery_trace), Some(&recovery_packet))
        .unwrap();
    db.upsert_request_summary(&RequestSummary {
        recovery_trace: Some(recovery_trace),
        recovery_packet: Some(recovery_packet),
        ..request_summary_fixture("req-clear", "job-clear", now)
    })
    .unwrap();
    db.insert_artifact(
        &ArtifactMetadata {
            artifact_id: "artifact-clear".to_owned(),
            conversation_id: Some("conv-clear".to_owned()),
            message_id: Some("msg-clear".to_owned()),
            file_name: "artifact-clear.txt".to_owned(),
            content_type: "text/plain".to_owned(),
            size_bytes: 8,
            sha256: "sha-clear".to_owned(),
            storage_path: artifact_path.display().to_string(),
        },
        now,
    )
    .unwrap();
    db.link_job_artifact("job-clear", "artifact-clear").unwrap();
    db.insert_failure_bundle(&FailureBundle {
        bundle_id: "bundle-clear".to_owned(),
        error_code: shared_protocol::ErrorCode::ProviderGenerationFailedNonretryable,
        message: "failure".to_owned(),
        created_at: now,
        url: None,
        mode: None,
        screenshot_path: None,
        dom_snapshot_path: None,
        aria_snapshot_path: None,
        logs_path: None,
        strategy_attempts: vec![],
        capability_matrix: CapabilityMatrix::default(),
        gateway_request: None,
    })
    .unwrap();

    let cleared = db.clear_all_state().unwrap();

    assert_eq!(cleared.result.sessions_deleted, 1);
    assert_eq!(cleared.result.conversations_deleted, 1);
    assert_eq!(cleared.result.jobs_deleted, 1);
    assert_eq!(cleared.result.job_details_deleted, 1);
    assert_eq!(cleared.result.request_history_deleted, 1);
    assert_eq!(cleared.result.artifacts_deleted, 1);
    assert_eq!(cleared.result.artifact_links_deleted, 1);
    assert_eq!(cleared.result.failure_bundles_deleted, 1);
    assert_eq!(
        cleared.artifact_storage_paths,
        vec![artifact_path.display().to_string()]
    );

    for table in [
        "sessions",
        "conversations",
        "jobs",
        "job_details",
        "request_history",
        "artifacts",
        "job_artifacts",
        "failure_bundles",
    ] {
        let count: i64 = db
            .conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0, "{table} should be empty");
    }
    assert_eq!(
        db.migration_count().unwrap(),
        migrations::MIGRATIONS.len() as u32
    );
    assert!(
        artifact_path.exists(),
        "database clear does not delete files"
    );
}
