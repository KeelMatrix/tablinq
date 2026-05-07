use super::*;
use crate::routes::{
    V1_BROWSER_ATTACH_PATH, V1_BROWSER_CONNECTION_INFO_PATH, V1_BROWSER_DETACH_PATH,
    V1_BROWSER_PROFILE_INFO_PATH, V1_CONFIG_PATH, V1_DOCTOR_PATH, V1_JOBS_PATH, V1_REQUESTS_PATH,
    V1_STATUS_PATH,
};
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
    assert!(json["request_id"].as_str().is_some());
    assert_eq!(json["error"]["code"], "payload_too_large");
    assert_eq!(
        json["error"]["message"],
        "request body exceeds gateway limit"
    );

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
    assert!(json["request_id"].as_str().is_some());
    assert_eq!(json["error"]["code"], "bad_request");
    assert_eq!(json["error"]["message"], "unsupported Transfer-Encoding");

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
    assert_eq!(json["error"]["code"], "bad_request");
    assert_eq!(
        json["error"]["message"],
        "request line exceeds gateway limit"
    );

    let long_header = format!(
        "GET /health HTTP/1.1\r\nX-Long: {}\r\n\r\n",
        "a".repeat(MAX_HTTP_HEADER_LINE_BYTES)
    );
    let response = raw_http_request(&snapshot, long_header.as_bytes());
    assert_eq!(response.status_code, 400);
    let json: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(json["error"]["code"], "bad_request");
    assert_eq!(
        json["error"]["message"],
        "HTTP header line exceeds gateway limit"
    );

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
    assert!(json["request_id"].as_str().is_some());
    assert_eq!(json["error"]["code"], "invalid_api_key");

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
    assert!(json["request_id"].as_str().is_some());
    assert_eq!(json["error"]["code"], "payload_too_large");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn parse_and_payload_failures_include_request_ids() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let chunked = http_request(
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
    assert_eq!(chunked.status_code, 400);
    let chunked_json: Value = serde_json::from_slice(&chunked.body).unwrap();
    assert!(chunked_json["request_id"].as_str().is_some());
    assert_eq!(chunked_json["error"]["param"], Value::Null);
    assert_eq!(chunked_json["error"]["code"], "bad_request");

    let oversized = super::super::http_request_with_declared_content_length(
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
    assert_eq!(oversized.status_code, 413);
    let oversized_json: Value = serde_json::from_slice(&oversized.body).unwrap();
    assert!(oversized_json["request_id"].as_str().is_some());
    assert_eq!(oversized_json["error"]["param"], Value::Null);
    assert_eq!(oversized_json["error"]["code"], "payload_too_large");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn representative_v1_errors_use_nested_error_objects() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let cases = [
        (
            "POST",
            V1_FILES_PATH,
            vec![
                ("authorization".to_owned(), auth.clone()),
                ("content-type".to_owned(), "application/octet-stream".to_owned()),
            ],
            b"missing header".to_vec(),
        ),
        (
            "GET",
            "/v1/requests/req-missing",
            vec![("authorization".to_owned(), auth.clone())],
            Vec::new(),
        ),
        (
            "POST",
            V1_BROWSER_ATTACH_PATH,
            vec![
                ("authorization".to_owned(), auth.clone()),
                ("content-type".to_owned(), "application/json".to_owned()),
            ],
            br#"{"target_url":"https://chatgpt.com/""#.to_vec(),
        ),
        (
            "GET",
            "/v1/requests?limit=0",
            vec![("authorization".to_owned(), auth.clone())],
            Vec::new(),
        ),
    ];

    for (method, path, headers, body_bytes) in cases {
        let header_refs = headers
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
            .collect::<Vec<_>>();
        let response = http_request(
            &snapshot.host,
            snapshot.port,
            method,
            path,
            &header_refs,
            &body_bytes,
            5_000,
        )
        .unwrap();
        assert!(response.status_code >= 400, "{method} {path}");
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert!(body["request_id"].as_str().is_some(), "{method} {path}");
        assert!(body["error"].is_object(), "{method} {path}");
        assert!(!body["error"].is_string(), "{method} {path}");
        assert_eq!(body["error"]["param"], Value::Null, "{method} {path}");
    }

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn known_v1_paths_return_405_with_allow_headers_and_request_ids() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    for (method, path, allow) in [
        ("POST", V1_MODELS_PATH, "GET"),
        ("POST", V1_CAPABILITIES_PATH, "GET"),
        ("POST", V1_PROVIDER_PATH, "GET"),
        ("GET", V1_CHAT_COMPLETIONS_PATH, "POST"),
        ("PUT", V1_FILES_PATH, "GET, POST"),
        ("POST", V1_ARTIFACTS_PATH, "GET"),
        ("POST", V1_STATUS_PATH, "GET"),
        ("POST", V1_DOCTOR_PATH, "GET"),
        ("POST", V1_CONFIG_PATH, "GET"),
        ("POST", V1_JOBS_PATH, "GET"),
        ("POST", V1_REQUESTS_PATH, "GET"),
        ("POST", V1_CONVERSATIONS_PATH, "GET"),
        ("GET", V1_BROWSER_ATTACH_PATH, "POST"),
        ("GET", V1_BROWSER_DETACH_PATH, "POST"),
        ("POST", V1_BROWSER_PROFILE_INFO_PATH, "GET"),
        ("POST", V1_BROWSER_CONNECTION_INFO_PATH, "GET"),
    ] {
        let response = http_request(
            &snapshot.host,
            snapshot.port,
            method,
            path,
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 405, "{method} {path}");
        assert_eq!(response.reason_phrase, "Method Not Allowed");
        assert_eq!(
            response.headers.get("allow").map(String::as_str),
            Some(allow),
            "{method} {path}"
        );
        let json: Value = serde_json::from_slice(&response.body).unwrap();
        assert!(json["request_id"].as_str().is_some(), "{method} {path}");
        assert_eq!(
            json["error"]["code"], "method_not_allowed",
            "{method} {path}"
        );
    }

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}
