use super::*;
use crate::uploads::MAX_STAGED_UPLOAD_BYTES;

#[test]
fn upload_store_cleans_up_expired_staged_files() {
    let dir = tempdir().unwrap();
    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let store = UploadStore::load(&config).unwrap();
    let metadata = store
        .stage_upload("..\\unsafe?.txt", Some("text/plain"), b"hello")
        .unwrap();
    assert_eq!(metadata.file_name, "unsafe_.txt");

    let upload_dir = gateway_uploads_dir(&config).join(&metadata.file_id);
    let expired = StagedFileMetadata {
        expires_at: Utc::now() - chrono::Duration::seconds(1),
        ..metadata
    };
    fs::write(
        staged_metadata_path(&upload_dir),
        serde_json::to_vec_pretty(&expired).unwrap(),
    )
    .unwrap();

    store.cleanup_expired().unwrap();
    assert!(!upload_dir.exists());
}

#[test]
fn upload_store_rejects_files_larger_than_twenty_mib() {
    let dir = tempdir().unwrap();
    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let store = UploadStore::load(&config).unwrap();
    let body = vec![0_u8; MAX_STAGED_UPLOAD_BYTES + 1];

    let err = store
        .stage_upload("oversize.bin", Some("application/octet-stream"), &body)
        .unwrap_err();
    assert!(format!("{err:#}").contains("20 MiB"));
}

#[test]
fn files_upload_endpoint_stages_files_and_research_accepts_file_ids() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let upload = upload_file(
        &snapshot,
        &auth,
        "brief.txt",
        "text/plain",
        b"customer brief",
    );
    assert_eq!(upload.status_code, 201);
    let upload_body: Value = serde_json::from_slice(&upload.body).unwrap();
    let file_id = upload_body["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(upload_body["data"]["file_name"], "brief.txt");
    assert_eq!(upload_body["data"]["content_type"], "text/plain");
    assert_eq!(upload_body["data"]["size_bytes"], 14);
    assert!(upload_body["data"]["sha256"].as_str().is_some());
    assert!(
        upload_body["data"]["linked_request_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let first_body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "tablinq_file_ids": [file_id],
        "messages": [
            { "role": "user", "content": "summarize the upload" }
        ]
    }))
    .unwrap();
    let first = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        &first_body,
        20_000,
    )
    .unwrap();
    assert_eq!(first.status_code, 200);
    let first_json: Value = serde_json::from_slice(&first.body).unwrap();
    assert!(
        first_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap()
            .contains("brief.txt")
    );
    let conversation_id = first_json["tablinq_conversation_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let second_body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "tablinq_file_ids": [file_id],
        "tablinq_conversation_id": conversation_id,
        "messages": [
            { "role": "user", "content": "reuse the same upload again" }
        ]
    }))
    .unwrap();
    let second = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-request-id", "req-file-reuse-2"),
        ],
        &second_body,
        20_000,
    )
    .unwrap();
    assert_eq!(second.status_code, 200);

    let detail = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        &format!("{V1_FILES_PATH}/{file_id}"),
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(detail.status_code, 200);
    let detail_json: Value = serde_json::from_slice(&detail.body).unwrap();
    assert_eq!(detail_json["data"]["id"], file_id);
    assert!(
        detail_json["data"]["linked_request_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "req-file-reuse-2")
    );
    assert!(
        detail_json["data"]["linked_conversation_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.as_str().is_some())
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn files_upload_endpoint_rejects_files_larger_than_twenty_mib() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request_with_declared_content_length(
        &snapshot,
        &auth,
        "oversize.bin",
        "application/octet-stream",
        MAX_STAGED_UPLOAD_BYTES + 1,
    );
    assert_eq!(response.status_code, 413);
    let json: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(json["error"], "file exceeds 20 MiB limit");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn paperclip_like_http_flow_supports_files_and_artifacts() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let upload = upload_file(
        &snapshot,
        &auth,
        "paperclip-source.txt",
        "text/plain",
        b"paperclip source text",
    );
    assert_eq!(upload.status_code, 201);
    let upload_body: Value = serde_json::from_slice(&upload.body).unwrap();
    let file_id = upload_body["data"]["id"].as_str().unwrap().to_owned();

    let body = paperclip_request_body(
        "tablinq-research",
        "produce a summary report from the staged file",
        false,
        &[file_id.as_str()],
    );
    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-client-id", "paperclip"),
        ],
        &body,
        60_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    let job_id = body["tablinq_job_id"].as_str().unwrap().to_owned();
    assert!(body["warnings"].as_array().unwrap().iter().any(
        |value| value.as_str() == Some("usage is unavailable for the browser-backed provider")
    ));

    let artifacts = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_ARTIFACTS_PATH,
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(artifacts.status_code, 200);
    let artifacts_body: Value = serde_json::from_slice(&artifacts.body).unwrap();
    let artifact = artifacts_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["linked_job_ids"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == &Value::String(job_id.clone()))
        })
        .unwrap();
    let content = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        artifact["content_url"].as_str().unwrap(),
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(content.status_code, 200);
    assert!(
        String::from_utf8(content.body)
            .unwrap()
            .contains("paperclip-source.txt")
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn provider_smoke_covers_chat_files_artifacts_and_cancel_flow() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let chat = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        &serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "provider smoke ask" }]
        }))
        .unwrap(),
        20_000,
    )
    .unwrap();
    assert_eq!(chat.status_code, 200);

    let upload = upload_file(
        &snapshot,
        &auth,
        "smoke.txt",
        "text/plain",
        b"provider smoke file",
    );
    assert_eq!(upload.status_code, 201);
    let upload_body: Value = serde_json::from_slice(&upload.body).unwrap();
    let file_id = upload_body["data"]["id"].as_str().unwrap().to_owned();

    let artifact_response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        &paperclip_request_body(
            "tablinq-research",
            "create a smoke artifact",
            false,
            &[&file_id],
        ),
        20_000,
    )
    .unwrap();
    assert_eq!(artifact_response.status_code, 200);
    let artifact_response_body: Value = serde_json::from_slice(&artifact_response.body).unwrap();
    let request_id = artifact_response_body["request_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let artifacts = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_ARTIFACTS_PATH,
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(artifacts.status_code, 200);
    let artifacts_body: Value = serde_json::from_slice(&artifacts.body).unwrap();
    let artifact_id = artifacts_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["linked_request_ids"].as_array().is_some_and(|linked| {
                linked
                    .iter()
                    .any(|value| value == &Value::String(request_id.clone()))
            })
        })
        .and_then(|item| item["id"].as_str())
        .unwrap()
        .to_owned();

    let artifact_detail = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        &format!("{V1_ARTIFACTS_PATH}/{artifact_id}"),
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(artifact_detail.status_code, 200);

    let artifact_content = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        &format!("{V1_ARTIFACTS_PATH}/{artifact_id}/content"),
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(artifact_content.status_code, 200);
    assert!(
        String::from_utf8(artifact_content.body)
            .unwrap()
            .contains("smoke.txt")
    );

    let cancel_body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "messages": [{ "role": "user", "content": "cancel this smoke request" }]
    }))
    .unwrap();
    let host = snapshot.host.clone();
    let auth_for_thread = auth.clone();
    let running = thread::spawn(move || {
        http_request(
            &host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-provider-smoke-cancel"),
            ],
            &cancel_body,
            20_000,
        )
        .unwrap()
    });

    thread::sleep(Duration::from_millis(500));
    let cancel = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/requests/req-provider-smoke-cancel/cancel",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(cancel.status_code, 200);
    let running = running.join().unwrap();
    assert_eq!(running.status_code, 409);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn paperclip_like_requests_report_missing_staged_file_errors_cleanly() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let body = paperclip_request_body(
        "tablinq-instant",
        "read the staged file",
        false,
        &["file-missing"],
    );
    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-client-id", "paperclip"),
        ],
        &body,
        20_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 400);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["error"]["code"], "file_not_found");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]

fn artifacts_endpoints_list_metadata_and_stream_content() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let upload = upload_file(
        &snapshot,
        &auth,
        "report.txt",
        "text/plain",
        b"artifact seed",
    );
    assert_eq!(upload.status_code, 201);
    let upload_body: Value = serde_json::from_slice(&upload.body).unwrap();
    let file_id = upload_body["data"]["id"].as_str().unwrap().to_owned();

    let body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "tablinq_file_ids": [file_id],
        "messages": [
            { "role": "user", "content": "produce a downloadable report" }
        ]
    }))
    .unwrap();
    let completion = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        &body,
        20_000,
    )
    .unwrap();
    assert_eq!(completion.status_code, 200);
    let completion_body: Value = serde_json::from_slice(&completion.body).unwrap();
    let job_id = completion_body["tablinq_job_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let request_id = completion_body["request_id"].as_str().unwrap().to_owned();

    let artifacts = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_ARTIFACTS_PATH,
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(artifacts.status_code, 200);
    let artifacts_body: Value = serde_json::from_slice(&artifacts.body).unwrap();
    let artifact = artifacts_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["linked_job_ids"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == &Value::String(job_id.clone()))
        })
        .unwrap();
    let artifact_id = artifact["id"].as_str().unwrap().to_owned();

    let detail = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        &format!("{V1_ARTIFACTS_PATH}/{artifact_id}"),
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(detail.status_code, 200);
    let detail_body: Value = serde_json::from_slice(&detail.body).unwrap();
    assert_eq!(detail_body["data"]["id"], artifact_id);
    assert!(
        detail_body["data"]["linked_job_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == &Value::String(job_id.clone()))
    );
    assert!(
        detail_body["data"]["linked_request_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == &Value::String(request_id.clone()))
    );
    assert!(detail_body["data"]["created_at"].as_str().is_some());
    assert!(detail_body["data"]["sha256"].as_str().is_some());

    let content = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        &format!("{V1_ARTIFACTS_PATH}/{artifact_id}/content"),
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(content.status_code, 200);
    let text = String::from_utf8(content.body).unwrap();
    assert!(text.contains("report.txt"));

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn files_endpoint_keeps_duplicate_original_names_as_separate_staged_ids() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let first = upload_file(&snapshot, &auth, "same.txt", "text/plain", b"one");
    let second = upload_file(&snapshot, &auth, "same.txt", "text/plain", b"two");
    assert_eq!(first.status_code, 201);
    assert_eq!(second.status_code, 201);

    let files = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_FILES_PATH,
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(files.status_code, 200);
    let body: Value = serde_json::from_slice(&files.body).unwrap();
    let matching = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["file_name"] == "same.txt")
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 2);
    assert_ne!(matching[0]["id"], matching[1]["id"]);
    assert_eq!(matching[0]["file_name"], "same.txt");
    assert_eq!(matching[1]["file_name"], "same.txt");
    assert_ne!(matching[0]["sha256"], matching[1]["sha256"]);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}
