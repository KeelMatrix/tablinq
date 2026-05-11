use super::*;
use chrono::{TimeZone, Utc};
use shared_protocol::{
    ArtifactMetadata, JobKind, JobStatus, RequestExecutionKind, RequestSummary, RequestUsage,
};

const EVIDENCE_CONTENT_TYPE: &str = "application/tablinq-browser-evidence+json";

fn fixed_time() -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 5, 8, 9, 0, 0).unwrap()
}

fn seed_state_fixture(config: &AppConfig) {
    let db = Database::open(&config.paths.db_path).unwrap();
    let now = fixed_time();

    db.upsert_session(
        "session-shape",
        "Shape session",
        Some("https://chatgpt.com/c/conv-alpha"),
        now + chrono::Duration::minutes(20),
    )
    .unwrap();

    for (conversation_id, minutes) in [("conv-alpha", 10), ("conv-beta", 11), ("conv-gamma", 12)] {
        db.upsert_conversation(
            conversation_id,
            "session-shape",
            &format!("Conversation {conversation_id}"),
            Some(&format!("https://chatgpt.com/c/{conversation_id}")),
            Some(&format!("last prompt {conversation_id}")),
            Some(&format!("last response {conversation_id}")),
            None,
            now + chrono::Duration::minutes(minutes),
        )
        .unwrap();
    }

    for index in 1..=3 {
        let job_id = format!("job-shape-{index}");
        let request_id = format!("req-shape-{index}");
        let created_at = now + chrono::Duration::minutes(index);
        db.insert_job(
            &job_id,
            JobStatus::Completed,
            Some(&format!("job {index} complete")),
            Some("conv-alpha"),
            created_at,
            created_at + chrono::Duration::seconds(30),
        )
        .unwrap();
        db.upsert_job_detail(
            &job_id,
            JobKind::Ask,
            Some(&format!("request text {index}")),
            None,
        )
        .unwrap();
        db.update_job_output(
            &job_id,
            Some(&format!("response text {index}")),
            Some("https://chatgpt.com/c/conv-alpha"),
            None,
        )
        .unwrap();
        db.upsert_request_summary(&RequestSummary {
            request_id: request_id.clone(),
            job_id: job_id.clone(),
            model_id: Some("tablinq-instant".to_owned()),
            request_role: Some("primary".to_owned()),
            parent_request_id: None,
            execution_kind: RequestExecutionKind::Ask,
            status: JobStatus::Completed,
            cancelled: false,
            start_time: created_at,
            end_time: Some(created_at + chrono::Duration::seconds(30)),
            duration_ms: Some(30_000),
            queue_wait_ms: index as u64,
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
            warnings: vec!["usage unavailable".to_owned()],
        })
        .unwrap();

        let artifact_path = config
            .paths
            .state_root
            .join(format!("artifact-shape-{index}.txt"));
        fs::write(&artifact_path, format!("artifact {index}")).unwrap();
        let artifact = ArtifactMetadata {
            artifact_id: format!("artifact-shape-{index}"),
            conversation_id: Some("conv-alpha".to_owned()),
            message_id: Some(format!("msg-shape-{index}")),
            file_name: format!("artifact-shape-{index}.txt"),
            content_type: "text/plain".to_owned(),
            size_bytes: 10,
            sha256: format!("sha-shape-{index}"),
            storage_path: artifact_path.display().to_string(),
        };
        db.insert_artifact(&artifact, created_at + chrono::Duration::seconds(31))
            .unwrap();
        db.link_job_artifact(&job_id, &artifact.artifact_id)
            .unwrap();
    }

    let evidence_path = config.paths.state_root.join("evidence-shape-3.json");
    fs::write(&evidence_path, r#"{"request_id":"req-shape-3"}"#).unwrap();
    let evidence = ArtifactMetadata {
        artifact_id: "artifact-evidence-shape-3".to_owned(),
        conversation_id: Some("conv-alpha".to_owned()),
        message_id: None,
        file_name: "request-evidence-req-shape-3.json".to_owned(),
        content_type: EVIDENCE_CONTENT_TYPE.to_owned(),
        size_bytes: 28,
        sha256: "sha-evidence-shape-3".to_owned(),
        storage_path: evidence_path.display().to_string(),
    };
    db.insert_artifact(&evidence, now + chrono::Duration::minutes(4))
        .unwrap();
    db.link_job_artifact("job-shape-3", &evidence.artifact_id)
        .unwrap();
}

fn get_json(
    snapshot: &GatewayStatusSnapshot,
    auth: &str,
    path: &str,
    expected_status: u16,
) -> Value {
    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        path,
        &[("authorization", auth)],
        &[],
        10_000,
    )
    .unwrap();
    assert_json_status(&response, expected_status)
}

#[test]
fn empty_state_routes_keep_current_response_envelopes() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let requests = get_json(snapshot, &auth, "/v1/requests", 200);
    assert_eq!(requests["object"], "tablinq.request_list");
    assert_eq!(requests["data"].as_array().unwrap().len(), 0);
    assert_eq!(requests["pagination"]["limit"], 50);
    assert_eq!(requests["pagination"]["offset"], 0);
    assert_eq!(requests["pagination"]["has_more"], false);
    assert!(requests["pagination"]["next_offset"].is_null());

    let jobs = get_json(snapshot, &auth, "/v1/jobs", 200);
    assert_eq!(jobs["object"], "tablinq.job_list");
    assert_eq!(jobs["data"].as_array().unwrap().len(), 0);
    assert!(jobs["pagination"]["limit"].is_null());
    assert_eq!(jobs["pagination"]["returned"], 0);
    assert_eq!(jobs["pagination"]["has_more"], false);

    let conversations = get_json(snapshot, &auth, "/v1/conversations", 200);
    assert_eq!(conversations["object"], "tablinq.conversation_list");
    assert_eq!(conversations["data"].as_array().unwrap().len(), 0);
    assert!(conversations["pagination"]["limit"].is_null());
    assert_eq!(conversations["pagination"]["returned"], 0);

    let usage = get_json(snapshot, &auth, "/v1/usage", 200);
    assert_eq!(usage["object"], "tablinq.request_summary_list");
    assert_eq!(usage["data"].as_array().unwrap().len(), 0);
    assert_eq!(usage["limit"], 50);
    assert!(usage["warnings"].as_array().unwrap().iter().any(|item| {
        item.as_str()
            .is_some_and(|text| text.contains("token usage is unavailable"))
    }));

    let files = get_json(snapshot, &auth, "/v1/files", 200);
    assert_eq!(files["object"], "list");
    assert_eq!(files["data"].as_array().unwrap().len(), 0);
    assert!(files["pagination"]["limit"].is_null());
    assert_eq!(files["pagination"]["returned"], 0);

    let artifacts = get_json(snapshot, &auth, "/v1/artifacts", 200);
    assert_eq!(artifacts["object"], "list");
    assert_eq!(artifacts["data"].as_array().unwrap().len(), 0);
    assert!(artifacts["pagination"]["limit"].is_null());
    assert_eq!(artifacts["pagination"]["returned"], 0);

    for path in [
        "/v1/requests/req-missing",
        "/v1/requests/req-missing/evidence",
        "/v1/conversations/conv-missing",
        "/v1/conversations/conv-missing/messages",
        "/v1/conversations/conv-missing/snapshot",
        "/v1/files/file-missing",
        "/v1/artifacts/artifact-missing",
    ] {
        let body = get_json(snapshot, &auth, path, 404);
        assert!(body["request_id"].as_str().is_some(), "{path}");
        assert!(body["error"].is_object(), "{path}");
        assert!(body["error"]["message"].as_str().is_some(), "{path}");
        assert!(body["error"]["param"].is_null(), "{path}");
    }
}

#[test]
fn seeded_state_routes_keep_current_default_shapes_and_ordering() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    seed_state_fixture(&config);
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let first_upload = upload_file(snapshot, &auth, "one.txt", "text/plain", b"one");
    assert_eq!(first_upload.status_code, 201);
    let second_upload = upload_file(snapshot, &auth, "two.txt", "text/plain", b"two");
    assert_eq!(second_upload.status_code, 201);

    let requests = get_json(snapshot, &auth, "/v1/requests", 200);
    assert_eq!(requests["object"], "tablinq.request_list");
    assert!(requests["request_id"].as_str().is_some());
    assert_eq!(requests["data"].as_array().unwrap().len(), 3);
    assert_eq!(requests["data"][0]["id"], "req-shape-3");
    assert_eq!(requests["data"][0]["status"], "completed");
    assert_eq!(requests["data"][0]["metadata"]["job_id"], "job-shape-3");
    assert_eq!(requests["data"][0]["metadata"]["execution_kind"], "ask");
    assert_eq!(requests["pagination"]["limit"], 50);
    assert_eq!(requests["pagination"]["has_more"], false);
    assert!(requests["filter"]["status"].is_null());

    let request = get_json(snapshot, &auth, "/v1/requests/req-shape-3", 200);
    assert_eq!(request["object"], "tablinq.request_summary");
    assert_eq!(request["data"]["request_id"], "req-shape-3");
    assert_eq!(request["data"]["job_id"], "job-shape-3");
    assert_eq!(request["data"]["usage"]["status"], "unknown");
    assert_eq!(request["data"]["warnings"][0], "usage unavailable");

    let evidence = get_json(snapshot, &auth, "/v1/requests/req-shape-3/evidence", 200);
    assert_eq!(evidence["object"], "tablinq.request_evidence");
    assert_eq!(evidence["data"]["id"], "artifact-evidence-shape-3");
    assert_eq!(evidence["data"]["content_type"], EVIDENCE_CONTENT_TYPE);
    assert!(
        evidence["data"]["linked_request_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "req-shape-3")
    );

    let jobs = get_json(snapshot, &auth, "/v1/jobs", 200);
    assert_eq!(jobs["object"], "tablinq.job_list");
    assert_eq!(jobs["data"].as_array().unwrap().len(), 3);
    assert_eq!(jobs["data"][0]["job_id"], "job-shape-3");
    assert_eq!(jobs["data"][0]["conversation_id"], "conv-alpha");
    assert!(jobs["pagination"]["limit"].is_null());
    assert_eq!(jobs["pagination"]["returned"], 3);

    let conversations = get_json(snapshot, &auth, "/v1/conversations", 200);
    assert_eq!(conversations["object"], "tablinq.conversation_list");
    assert_eq!(conversations["data"].as_array().unwrap().len(), 3);
    assert_eq!(conversations["data"][0]["conversation_id"], "conv-gamma");
    assert_eq!(
        conversations["data"][0]["routes"]["messages"],
        "/v1/conversations/conv-gamma/messages"
    );
    assert!(conversations["pagination"]["limit"].is_null());
    assert_eq!(conversations["pagination"]["returned"], 3);

    let conversation = get_json(snapshot, &auth, "/v1/conversations/conv-alpha", 200);
    assert_eq!(conversation["object"], "tablinq.conversation");
    assert_eq!(conversation["data"]["conversation_id"], "conv-alpha");
    assert_eq!(conversation["data"]["latest_request_id"], "req-shape-3");
    assert_eq!(conversation["data"]["latest_job_id"], "job-shape-3");
    assert_eq!(conversation["data"]["counts"]["request_count"], 3);

    let messages = get_json(
        snapshot,
        &auth,
        "/v1/conversations/conv-alpha/messages",
        200,
    );
    assert_eq!(messages["object"], "tablinq.conversation_message_list");
    assert_eq!(messages["pagination"]["limit"], 50);
    assert_eq!(messages["pagination"]["offset"], 0);
    assert_eq!(messages["pagination"]["total_items"], 6);
    assert_eq!(messages["data"][0]["content"], "request text 1");
    assert_eq!(messages["data"][5]["content"], "response text 3");

    let snapshot_body = get_json(
        snapshot,
        &auth,
        "/v1/conversations/conv-alpha/snapshot",
        200,
    );
    assert_eq!(snapshot_body["object"], "tablinq.conversation_snapshot");
    assert_eq!(
        snapshot_body["data"]["conversation"]["conversation_id"],
        "conv-alpha"
    );
    assert_eq!(snapshot_body["data"]["latest_request_id"], "req-shape-3");
    assert_eq!(
        snapshot_body["data"]["recent_messages"]
            .as_array()
            .unwrap()
            .len(),
        6
    );

    let usage = get_json(snapshot, &auth, "/v1/usage", 200);
    assert_eq!(usage["object"], "tablinq.request_summary_list");
    assert_eq!(usage["data"].as_array().unwrap().len(), 3);
    assert_eq!(usage["data"][0]["request_id"], "req-shape-3");
    assert_eq!(usage["limit"], 50);

    let files = get_json(snapshot, &auth, "/v1/files", 200);
    assert_eq!(files["object"], "list");
    assert_eq!(files["data"].as_array().unwrap().len(), 2);
    assert!(files["pagination"]["limit"].is_null());
    assert!(
        files["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["file_name"] == "one.txt")
    );
    assert!(
        files["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["file_name"] == "two.txt")
    );

    let artifacts = get_json(snapshot, &auth, "/v1/artifacts", 200);
    assert_eq!(artifacts["object"], "list");
    assert_eq!(artifacts["data"].as_array().unwrap().len(), 4);
    assert!(artifacts["pagination"]["limit"].is_null());
    assert_eq!(artifacts["data"][0]["id"], "artifact-evidence-shape-3");
    assert_eq!(
        artifacts["data"][0]["content_url"],
        "/v1/artifacts/artifact-evidence-shape-3/content"
    );
}

#[test]
fn invalid_response_shape_query_parameters_return_structured_errors() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    seed_state_fixture(&config);
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let invalid_request_limit = get_json(snapshot, &auth, "/v1/requests?limit=not-a-number", 400);
    assert_eq!(
        invalid_request_limit["error"]["type"],
        "invalid_request_error"
    );
    assert_eq!(invalid_request_limit["error"]["code"], "invalid_request");
    assert_eq!(
        invalid_request_limit["error"]["message"],
        "limit must be a positive integer"
    );

    let invalid_messages_limit = get_json(
        snapshot,
        &auth,
        "/v1/conversations/conv-alpha/messages?limit=not-a-number",
        400,
    );
    assert_eq!(
        invalid_messages_limit["error"]["message"],
        "limit must be a positive integer"
    );

    let requests_unknown = get_json(snapshot, &auth, "/v1/requests?unknown=1", 200);
    assert_eq!(requests_unknown["data"].as_array().unwrap().len(), 3);
    assert_eq!(requests_unknown["pagination"]["limit"], 50);

    let invalid_offset = get_json(snapshot, &auth, "/v1/jobs?offset=-1", 400);
    assert_eq!(
        invalid_offset["error"]["message"],
        "offset must be a non-negative integer"
    );

    let invalid_view = get_json(snapshot, &auth, "/v1/conversations?view=everything", 400);
    assert_eq!(
        invalid_view["error"]["message"],
        "view must be one of summary, detail, full"
    );

    let invalid_include = get_json(
        snapshot,
        &auth,
        "/v1/requests/req-shape-3?include=everything",
        400,
    );
    assert_eq!(
        invalid_include["error"]["message"],
        "unsupported include: everything"
    );

    let messages_unknown = get_json(
        snapshot,
        &auth,
        "/v1/conversations/conv-alpha/messages?unknown=1",
        200,
    );
    assert_eq!(messages_unknown["pagination"]["limit"], 50);
    assert_eq!(messages_unknown["data"].as_array().unwrap().len(), 6);

    let invalid_compact = get_json(
        snapshot,
        &auth,
        "/v1/conversations/conv-alpha/snapshot?compact=not-bool",
        400,
    );
    assert_eq!(
        invalid_compact["error"]["message"],
        "compact must be true or false"
    );

    let invalid_summary = get_json(snapshot, &auth, "/v1/usage?summary=not-bool", 400);
    assert_eq!(
        invalid_summary["error"]["message"],
        "summary must be true or false"
    );

    let invalid_order = get_json(
        snapshot,
        &auth,
        "/v1/conversations/conv-alpha/messages?order=random",
        400,
    );
    assert_eq!(
        invalid_order["error"]["message"],
        "order must be one of asc, desc"
    );

    let artifacts_invalid_limit =
        get_json(snapshot, &auth, "/v1/artifacts?limit=not-a-number", 400);
    assert_eq!(
        artifacts_invalid_limit["error"]["message"],
        "limit must be a positive integer"
    );
}

#[test]
fn response_size_controls_paginate_compact_and_expand_state_routes() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    seed_state_fixture(&config);
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    for (name, body, expected_id) in [
        ("one.txt", b"one".as_slice(), "one.txt"),
        ("two.txt", b"two".as_slice(), "two.txt"),
        ("three.txt", b"three".as_slice(), "three.txt"),
    ] {
        let upload = upload_file(snapshot, &auth, name, "text/plain", body);
        assert_eq!(upload.status_code, 201);
        let upload_body: Value = serde_json::from_slice(&upload.body).unwrap();
        assert_eq!(upload_body["data"]["file_name"], expected_id);
    }

    let requests = get_json(
        snapshot,
        &auth,
        "/v1/requests?limit=1&offset=1&view=summary",
        200,
    );
    assert_eq!(requests["data"].as_array().unwrap().len(), 1);
    assert_eq!(requests["data"][0]["id"], "req-shape-2");
    assert_eq!(requests["pagination"]["limit"], 1);
    assert_eq!(requests["pagination"]["offset"], 1);
    assert_eq!(requests["pagination"]["returned"], 1);
    assert_eq!(requests["pagination"]["has_more"], true);
    assert_eq!(requests["pagination"]["next_offset"], 2);
    assert_eq!(requests["meta"]["view"], "summary");

    let request_full = get_json(snapshot, &auth, "/v1/requests/req-shape-3?view=full", 200);
    assert_eq!(request_full["data"]["usage"]["status"], "unknown");
    assert_eq!(request_full["meta"]["truncated"], false);

    let request_with_evidence = get_json(
        snapshot,
        &auth,
        "/v1/requests/req-shape-3?view=summary&include=evidence",
        200,
    );
    assert_eq!(
        request_with_evidence["data"]["evidence"]["id"],
        "artifact-evidence-shape-3"
    );
    assert_eq!(request_with_evidence["meta"]["truncated"], true);

    let evidence_page = get_json(
        snapshot,
        &auth,
        "/v1/requests/req-shape-3/evidence?limit=1",
        200,
    );
    assert_eq!(evidence_page["pagination"]["returned"], 1);
    assert_eq!(evidence_page["pagination"]["has_more"], false);

    let jobs = get_json(
        snapshot,
        &auth,
        "/v1/jobs?limit=1&offset=1&view=summary",
        200,
    );
    assert_eq!(jobs["data"].as_array().unwrap().len(), 1);
    assert_eq!(jobs["data"][0]["job_id"], "job-shape-2");
    assert_eq!(jobs["pagination"]["next_offset"], 2);

    let conversations = get_json(
        snapshot,
        &auth,
        "/v1/conversations?limit=1&offset=1&view=summary",
        200,
    );
    assert_eq!(conversations["data"].as_array().unwrap().len(), 1);
    assert_eq!(conversations["data"][0]["conversation_id"], "conv-beta");
    assert!(conversations["data"][0].get("last_prompt").is_none());
    assert_eq!(conversations["meta"]["truncated"], true);

    let messages = get_json(
        snapshot,
        &auth,
        "/v1/conversations/conv-alpha/messages?limit=2&offset=1",
        200,
    );
    assert_eq!(messages["pagination"]["returned"], 2);
    assert_eq!(messages["pagination"]["has_more"], true);
    assert_eq!(messages["pagination"]["next_offset"], 3);

    let messages_desc = get_json(
        snapshot,
        &auth,
        "/v1/conversations/conv-alpha/messages?limit=1&order=desc",
        200,
    );
    assert_eq!(messages_desc["data"][0]["content"], "response text 3");
    assert_eq!(messages_desc["meta"]["order"], "desc");

    let compact_snapshot = get_json(
        snapshot,
        &auth,
        "/v1/conversations/conv-alpha/snapshot?compact=true",
        200,
    );
    assert_eq!(compact_snapshot["meta"]["compact"], true);
    assert_eq!(compact_snapshot["meta"]["truncated"], true);
    assert_eq!(compact_snapshot["meta"]["max_items"], 3);
    assert_eq!(compact_snapshot["meta"]["returned_items"], 3);
    assert!(
        compact_snapshot["data"]
            .get("tracked_generated_artifacts")
            .is_none()
    );

    let full_snapshot = get_json(
        snapshot,
        &auth,
        "/v1/conversations/conv-alpha/snapshot?compact=false&view=full",
        200,
    );
    assert_eq!(full_snapshot["meta"]["truncated"], false);
    assert_eq!(
        full_snapshot["data"]["recent_messages"]
            .as_array()
            .unwrap()
            .len(),
        6
    );

    let usage = get_json(
        snapshot,
        &auth,
        "/v1/usage?summary=true&limit=1&offset=1",
        200,
    );
    assert_eq!(usage["data"].as_array().unwrap().len(), 1);
    assert_eq!(usage["data"][0]["request_id"], "req-shape-2");
    assert!(usage["data"][0].get("warnings").is_none());
    assert_eq!(usage["pagination"]["returned"], 1);

    let files = get_json(
        snapshot,
        &auth,
        "/v1/files?limit=1&offset=1&view=summary",
        200,
    );
    assert_eq!(files["data"].as_array().unwrap().len(), 1);
    assert!(files["data"][0].get("linked_request_ids").is_none());
    assert_eq!(files["pagination"]["returned"], 1);
    assert_eq!(files["pagination"]["has_more"], true);

    let artifacts = get_json(
        snapshot,
        &auth,
        "/v1/artifacts?limit=1&offset=1&view=summary",
        200,
    );
    assert_eq!(artifacts["data"].as_array().unwrap().len(), 1);
    assert!(artifacts["data"][0].get("linked_job_ids").is_none());
    assert_eq!(artifacts["pagination"]["returned"], 1);
    assert_eq!(artifacts["pagination"]["has_more"], true);
}
