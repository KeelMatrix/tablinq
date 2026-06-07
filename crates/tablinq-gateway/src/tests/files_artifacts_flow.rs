#![allow(unused_imports, dead_code)]
use super::*;

const UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS: u64 = 60_000;

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
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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
            UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
    )
    .unwrap();
    assert_eq!(response.status_code, 400);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["error"]["code"], "file_not_found");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn artifact_detail_and_content_missing_targets_return_nested_not_found_errors() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    for path in [
        "/v1/artifacts/artifact-missing",
        "/v1/artifacts/artifact-missing/content",
    ] {
        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            path,
            &[("authorization", auth.as_str())],
            &[],
            UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
        )
        .unwrap();
        assert_eq!(response.status_code, 404, "{path}");
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert!(body["request_id"].as_str().is_some(), "{path}");
        assert!(body["error"].is_object(), "{path}");
        assert_eq!(body["error"]["type"], "not_found_error", "{path}");
        assert_eq!(body["error"]["code"], "not_found", "{path}");
        assert_eq!(body["error"]["param"], Value::Null, "{path}");
    }

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}
