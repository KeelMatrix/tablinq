#![allow(unused_imports, dead_code)]
use super::*;
use crate::uploads::MAX_STAGED_UPLOAD_BYTES;

const UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS: u64 = 60_000;

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
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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

    let response = upload_with_declared_content_length(
        &snapshot,
        &auth,
        "oversize.bin",
        "application/octet-stream",
        MAX_STAGED_UPLOAD_BYTES + 1,
    );
    assert_eq!(response.status_code, 413);
    let json: Value = serde_json::from_slice(&response.body).unwrap();
    assert!(json["request_id"].as_str().is_some());
    assert!(json["error"].is_object());
    assert_eq!(json["error"]["type"], "invalid_request_error");
    assert_eq!(json["error"]["code"], "payload_too_large");
    assert_eq!(json["error"]["param"], Value::Null);
    assert_eq!(json["error"]["message"], "file exceeds 20 MiB limit");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn files_upload_endpoint_requires_x_file_name_header_with_nested_error() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_FILES_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/octet-stream"),
        ],
        b"hello",
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
    )
    .unwrap();
    assert_eq!(response.status_code, 400);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert!(body["request_id"].as_str().is_some());
    assert!(body["error"].is_object());
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["code"], "missing_file_name_header");
    assert_eq!(body["error"]["param"], Value::Null);
    assert_eq!(body["error"]["message"], "x-file-name header is required");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn file_detail_missing_target_returns_nested_not_found_error() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/files/file-missing",
        &[("authorization", auth.as_str())],
        &[],
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
    )
    .unwrap();
    assert_eq!(response.status_code, 404);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert!(body["request_id"].as_str().is_some());
    assert!(body["error"].is_object());
    assert_eq!(body["error"]["type"], "not_found_error");
    assert_eq!(body["error"]["code"], "not_found");
    assert_eq!(body["error"]["param"], Value::Null);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
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
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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
