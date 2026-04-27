use super::*;
use crate::uploads::MAX_STAGED_UPLOAD_BYTES;

#[test]
fn non_upload_body_over_limit_returns_413_before_body_read() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = super::super::http_request_with_declared_content_length(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        MAX_NON_UPLOAD_REQUEST_BODY_BYTES + 1,
        5_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 413);
    assert_eq!(response.reason_phrase, "Payload Too Large");
    let json: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(json["error"], "request body exceeds gateway limit");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn chunked_transfer_encoding_is_rejected_before_body_parsing() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("transfer-encoding", "chunked"),
        ],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 400);
    let json: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(json["error"], "bad_request");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn oversized_request_line_and_header_are_rejected_deterministically() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let (handle, snapshot) = spawn_gateway(config);

    let long_path = format!("/{}{}", "a".repeat(MAX_HTTP_REQUEST_LINE_BYTES), "\r\n");
    let long_request_line = format!("GET {long_path}Host: localhost\r\n\r\n");
    let response = raw_http_request(&snapshot, long_request_line.as_bytes());
    assert_eq!(response.status_code, 400);
    let json: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(json["error"], "bad_request");

    let long_header = format!(
        "GET /health HTTP/1.1\r\nX-Long: {}\r\n\r\n",
        "a".repeat(MAX_HTTP_HEADER_LINE_BYTES)
    );
    let response = raw_http_request(&snapshot, long_header.as_bytes());
    assert_eq!(response.status_code, 400);
    let json: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(json["error"], "bad_request");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn bearer_token_auth_accepts_valid_token_and_rejects_invalid_token() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let valid = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_FILES_PATH,
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(valid.status_code, 200);

    let invalid = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_FILES_PATH,
        &[("authorization", "Bearer wrong-token")],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(invalid.status_code, 401);
    let json: Value = serde_json::from_slice(&invalid.body).unwrap();
    assert_eq!(json["error"], "unauthorized");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn declared_upload_over_limit_preserves_twenty_mib_cap_response() {
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
    assert_eq!(response.reason_phrase, "Payload Too Large");
    let json: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(json["error"], "file exceeds 20 MiB limit");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}
