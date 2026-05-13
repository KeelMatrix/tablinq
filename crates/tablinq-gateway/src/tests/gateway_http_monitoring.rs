use super::*;
use gptd::db::{GATEWAY_HTTP_STREAMING_RESPONSE_REASON, GatewayHttpRequestRecord};
use sha2::{Digest, Sha256};
use std::path::Path;

#[test]
fn parse_failure_requests_are_captured_for_gateway_http_monitoring() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let gateway = RunningGateway::start(config);

    let response = raw_http_request(
        &gateway.snapshot,
        b"POST /v1/chat/completions HTTP/1.1\r\n\
          Host: localhost\r\n\
          X-Request-Id: req-monitor-parse\r\n\
          Authorization: Bearer secret\r\n\
          Transfer-Encoding: chunked\r\n\
          \r\n",
    );
    assert_eq!(response.status_code, 400);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["request_id"], "req-monitor-parse");
    assert_eq!(body["error"]["code"], "bad_request");

    let record = wait_for_monitoring_record(&state_root, "req-monitor-parse");
    assert_eq!(record.method, "POST");
    assert_eq!(record.path, V1_CHAT_COMPLETIONS_PATH);
    assert_eq!(record.route.as_deref(), Some("ChatCompletions"));
    assert_eq!(record.response_status_code, Some(400));
    assert!(
        record
            .parse_error
            .as_deref()
            .is_some_and(|error| error.contains("unsupported Transfer-Encoding"))
    );
    let headers = parse_headers(record.request_headers_json.as_deref().unwrap());
    assert_eq!(header_value(&headers, "authorization"), Some("[redacted]"));
}

#[test]
fn normal_json_api_request_response_is_captured_without_request_history_rows() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);

    let response = http_request(
        &gateway.snapshot.host,
        gateway.snapshot.port,
        "GET",
        HEALTH_PATH,
        &[
            ("authorization", auth.as_str()),
            ("x-request-id", "req-monitor-json"),
        ],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 200);

    let record = wait_for_monitoring_record(&state_root, "req-monitor-json");
    assert_eq!(record.method, "GET");
    assert_eq!(record.path, HEALTH_PATH);
    assert_eq!(record.route.as_deref(), Some("Health"));
    assert_eq!(record.request_body_text.as_deref(), Some(""));
    assert_eq!(record.request_body_sha256, Some(sha256_hex(&[])));
    assert!(!record.request_body_truncated);
    assert_eq!(record.response_status_code, Some(200));
    assert_eq!(
        record.response_body_text.as_deref(),
        Some(String::from_utf8_lossy(&response.body).as_ref())
    );
    assert_eq!(
        record.response_body_sha256,
        Some(sha256_hex(&response.body))
    );
    assert!(!record.response_body_truncated);
    assert!(record.response_capture_omitted_reason.is_none());

    let headers = parse_headers(record.request_headers_json.as_deref().unwrap());
    assert_eq!(header_value(&headers, "authorization"), Some("[redacted]"));
    assert_eq!(
        header_value(&headers, "x-request-id"),
        Some("req-monitor-json")
    );

    let db = db_for_state_root(&state_root);
    assert_eq!(db.gateway_http_request_count().unwrap(), 1);
    assert!(db.list_request_summaries(10).unwrap().requests.is_empty());
}

#[test]
fn streaming_sse_response_is_unchanged_and_body_capture_is_omitted() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);

    let body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "stream": true,
        "messages": [{ "role": "user", "content": "monitor the stream" }]
    }))
    .unwrap();
    let response = http_request(
        &gateway.snapshot.host,
        gateway.snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-request-id", "req-monitor-stream"),
        ],
        &body,
        20_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 200);
    assert_eq!(
        response.headers.get("content-type").map(String::as_str),
        Some("text/event-stream")
    );
    let sse_text = String::from_utf8(response.body).unwrap();
    assert!(sse_text.contains("data: [DONE]\n\n"));
    assert!(sse_text.contains("\"object\":\"chat.completion.chunk\""));

    let record = wait_for_monitoring_record(&state_root, "req-monitor-stream");
    assert_eq!(record.response_status_code, Some(200));
    assert_eq!(
        record.response_capture_omitted_reason.as_deref(),
        Some(GATEWAY_HTTP_STREAMING_RESPONSE_REASON)
    );
    assert!(record.response_body_text.is_none());
    assert!(record.response_body_sha256.is_none());
    assert!(!record.response_body_truncated);
    let headers = parse_headers(record.response_headers_json.as_deref().unwrap());
    assert_eq!(
        header_value(&headers, "content-type"),
        Some("text/event-stream")
    );

    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

fn wait_for_monitoring_record(state_root: &Path, request_id: &str) -> GatewayHttpRequestRecord {
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let db = db_for_state_root(state_root);
        if let Some(record) = db.latest_gateway_http_request(request_id).unwrap()
            && record.finished_at.is_some()
        {
            return record;
        }
        assert!(
            Instant::now() < deadline,
            "monitoring record {request_id} was not written"
        );
        thread::sleep(std::time::Duration::from_millis(25));
    }
}

fn db_for_state_root(state_root: &Path) -> Database {
    let config = AppConfig::load_or_create(Some(state_root)).unwrap();
    Database::open(&config.paths.db_path).unwrap()
}

fn parse_headers(headers: &str) -> Vec<Value> {
    serde_json::from_str(headers).unwrap()
}

fn header_value<'a>(headers: &'a [Value], name: &str) -> Option<&'a str> {
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
