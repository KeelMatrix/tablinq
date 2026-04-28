#![allow(unused_imports, dead_code)]
use super::*;
use crate::uploads::MAX_STAGED_UPLOAD_BYTES;

const UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS: u64 = 60_000;

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
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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
    let artifact_id = artifact["id"].as_str().unwrap().to_owned();

    let detail = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        &format!("{V1_ARTIFACTS_PATH}/{artifact_id}"),
        &[("authorization", auth.as_str())],
        &[],
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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
        UPLOAD_ARTIFACT_HTTP_TIMEOUT_MS,
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
