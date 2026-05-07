use super::*;

#[test]
fn request_summary_endpoint_persists_latency_and_unknown_usage_for_success() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "messages": [{ "role": "user", "content": "summarize latency" }]
    }))
    .unwrap();
    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-request-id", "req-metrics-success"),
        ],
        &body,
        20_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 200);

    let summary = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/requests/req-metrics-success",
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(summary.status_code, 200);
    let summary_body: Value = serde_json::from_slice(&summary.body).unwrap();
    assert_eq!(summary_body["data"]["request_id"], "req-metrics-success");
    assert_eq!(summary_body["data"]["status"], "completed");
    assert_eq!(summary_body["data"]["execution_kind"], "ask");
    assert_eq!(summary_body["data"]["cancelled"], false);
    assert!(summary_body["data"]["duration_ms"].as_u64().is_some());
    assert!(summary_body["data"]["queue_wait_ms"].as_u64().is_some());
    assert_eq!(summary_body["data"]["usage"]["status"], "unknown");
    assert!(
        summary_body["data"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.as_str()
                == Some("exact token usage is unavailable for the browser-backed provider"))
    );

    let usage = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_USAGE_PATH,
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(usage.status_code, 200);
    let usage_body: Value = serde_json::from_slice(&usage.body).unwrap();
    assert_eq!(usage_body["object"], "tablinq.request_summary_list");
    assert!(
        usage_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value["request_id"] == "req-metrics-success")
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn requests_endpoint_lists_paginated_request_summaries_with_status_filter() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    for (request_id, prompt) in [
        ("req-list-1", "first listed request"),
        ("req-list-2", "second listed request"),
    ] {
        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", request_id),
            ],
            &serde_json::to_vec(&json!({
                "model": "tablinq-instant",
                "messages": [{ "role": "user", "content": prompt }]
            }))
            .unwrap(),
            20_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 200);
    }

    let page = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/requests?status=completed&limit=1&offset=0",
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(page.status_code, 200);
    let page_body: Value = serde_json::from_slice(&page.body).unwrap();
    assert_eq!(page_body["object"], "tablinq.request_list");
    assert_eq!(page_body["filter"]["status"], "completed");
    assert_eq!(page_body["pagination"]["limit"], 1);
    assert_eq!(page_body["pagination"]["offset"], 0);
    assert_eq!(page_body["pagination"]["next_offset"], 1);
    assert_eq!(page_body["data"][0]["id"], "req-list-2");
    assert_eq!(page_body["data"][0]["status"], "completed");
    assert_eq!(page_body["data"][0]["model"], "tablinq-instant");
    assert_eq!(page_body["data"][0]["metadata"]["execution_kind"], "ask");

    let next_page = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/requests?status=completed&limit=1&offset=1",
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(next_page.status_code, 200);
    let next_body: Value = serde_json::from_slice(&next_page.body).unwrap();
    assert_eq!(next_body["data"][0]["id"], "req-list-1");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn timed_out_requests_capture_browser_evidence_and_expose_it_by_request_id() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}?scenario=timeout"));
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.ask_timeout_ms = Some(1_000);
    }
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
            ("x-request-id", "req-evidence-timeout"),
        ],
        &serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "capture timeout evidence" }]
        }))
        .unwrap(),
        20_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 504);

    let evidence = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/requests/req-evidence-timeout/evidence",
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(evidence.status_code, 200);
    let body: Value = serde_json::from_slice(&evidence.body).unwrap();
    assert_eq!(body["object"], "tablinq.request_evidence");
    assert_eq!(
        body["data"]["content_type"],
        "application/tablinq-browser-evidence+json"
    );
    assert!(
        body["data"]["linked_request_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "req-evidence-timeout")
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn request_summary_records_recovered_status_when_the_gateway_client_disconnects() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "stream": true,
        "messages": [{ "role": "user", "content": "recover after the gateway client disconnects" }]
    }))
    .unwrap();
    let mut stream = TcpStream::connect((&snapshot.host[..], snapshot.port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(
        stream,
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: {}\r\nContent-Type: application/json\r\nX-Request-Id: req-client-drop\r\nContent-Length: {}\r\n\r\n",
        V1_CHAT_COMPLETIONS_PATH,
        snapshot.host,
        snapshot.port,
        auth,
        body.len()
    )
    .unwrap();
    stream.write_all(&body).unwrap();
    stream.flush().unwrap();
    let mut buffer = [0u8; 512];
    let _ = stream.read(&mut buffer).unwrap_or(0);
    let _ = stream.shutdown(std::net::Shutdown::Both);
    drop(stream);

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let summary = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            "/v1/requests/req-client-drop",
            &[("authorization", auth.as_str())],
            &[],
            10_000,
        )
        .unwrap();
        if summary.status_code == 200 {
            let summary_body: Value = serde_json::from_slice(&summary.body).unwrap();
            assert_eq!(summary_body["data"]["status"], "recovered");
            assert!(summary_body["data"]["interrupted_at"].as_str().is_some());
            assert!(summary_body["data"]["recovered_at"].as_str().is_some());
            assert!(
                summary_body["data"]["interruption_reason"]
                    .as_str()
                    .unwrap()
                    .contains("client disconnected")
            );
            let config = AppConfig::load_or_create(Some(&state_root)).unwrap();
            let db = Database::open(&config.paths.db_path).unwrap();
            let request = db.request_summary("req-client-drop").unwrap().unwrap();
            assert_eq!(request.status, JobStatus::Recovered);
            assert!(request.recovered_at.is_some());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "expected a recovered request summary after the client disconnected"
        );
        thread::sleep(Duration::from_millis(50));
    }

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn request_summary_endpoint_records_failed_requests_with_latency() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-failure"));
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "messages": [{ "role": "user", "content": "record a failed summary" }]
    }))
    .unwrap();
    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-request-id", "req-metrics-failure"),
        ],
        &body,
        20_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 502);

    let summary = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/requests/req-metrics-failure",
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(summary.status_code, 200);
    let summary_body: Value = serde_json::from_slice(&summary.body).unwrap();
    assert_eq!(summary_body["data"]["status"], "failed");
    assert_eq!(summary_body["data"]["execution_kind"], "research");
    assert_eq!(summary_body["data"]["cancelled"], false);
    assert!(summary_body["data"]["duration_ms"].as_u64().is_some());
    assert!(summary_body["data"]["end_time"].as_str().is_some());

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_maps_capability_errors_to_openai_shape() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?missing=search"));
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let body = serde_json::to_vec(&json!({
        "model": "tablinq-search",
        "messages": [
            { "role": "user", "content": "search for release notes" }
        ]
    }))
    .unwrap();
    let response = http_request(
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
    assert_eq!(response.status_code, 400);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["error"]["code"], "capability_missing");
    assert_eq!(body["tablinq_refusal_reason"], "capability_missing");
    assert!(body["tablinq_failure_bundle_id"].as_str().is_some());
    assert!(
        body["tablinq_guidance"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value
                .as_str()
                .is_some_and(|text| text.contains("refuses requests")))
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn openai_auth_errors_include_refusal_guidance() {
    let err =
        anyhow::anyhow!("auth required: attached browser-backed ChatGPT tab is not authenticated");
    let response = map_openai_error("req-auth-expired", &err, Some("bundle-auth-1"));
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(response.status_code, 503);
    assert_eq!(body["error"]["code"], "auth_required");
    assert_eq!(body["tablinq_refusal_reason"], "auth_required");
    assert_eq!(body["tablinq_failure_bundle_id"], "bundle-auth-1");
    assert!(
        body["tablinq_guidance"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value
                .as_str()
                .is_some_and(|text| text.contains("/v1/browser/connection-info")
                    || text.contains("signed-in ChatGPT tab")))
    );
}

#[test]
fn gateway_rejects_missing_and_wrong_bearer_tokens_for_openai_routes() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let (handle, snapshot) = spawn_gateway(config);
    let body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "messages": [{ "role": "user", "content": "reject unauthorized OpenAI-shaped requests" }]
    }))
    .unwrap();

    let missing = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[("content-type", "application/json")],
        &body,
        5_000,
    )
    .unwrap();
    assert_eq!(missing.status_code, 401);
    let missing_body: Value = serde_json::from_slice(&missing.body).unwrap();
    assert_eq!(missing_body["error"]["code"], "invalid_api_key");

    let wrong = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", "Bearer not-the-real-token"),
            ("content-type", "application/json"),
        ],
        &body,
        5_000,
    )
    .unwrap();
    assert_eq!(wrong.status_code, 401);
    let wrong_body: Value = serde_json::from_slice(&wrong.body).unwrap();
    assert_eq!(wrong_body["error"]["code"], "invalid_api_key");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn chat_completions_return_auth_required_when_browser_session_is_signed_out() {
    let response = map_openai_error(
        "req-auth-expired",
        &anyhow::anyhow!("auth required: attached browser-backed ChatGPT tab is not authenticated"),
        Some("bundle-auth-1"),
    );
    assert_eq!(response.status_code, 503);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["error"]["code"], "auth_required");
    assert_eq!(body["tablinq_refusal_reason"], "auth_required");
    assert_eq!(body["tablinq_failure_bundle_id"], "bundle-auth-1");
}

#[test]
fn request_role_fields_flow_into_failure_bundles_and_request_summaries() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=timeout"));
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.ask_timeout_ms = Some(1_000);
    }
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let body = openclaude_request_body(
        "tablinq-instant",
        "fail and persist branch request role metadata",
        "branch",
        false,
        Some("req-parent-1"),
    );
    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-request-id", "req-role-timeout"),
        ],
        &body,
        20_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 504);

    let summary_body = request_summary_json(&snapshot, &auth, "req-role-timeout");
    assert_eq!(summary_body["data"]["request_role"], "branch");
    assert_eq!(summary_body["data"]["parent_request_id"], "req-parent-1");

    let gateway_request = latest_failure_bundle_request(&state_root);
    assert_eq!(gateway_request.request_id, "req-role-timeout");
    assert_eq!(gateway_request.request_path, V1_CHAT_COMPLETIONS_PATH);
    assert_eq!(gateway_request.model.as_deref(), Some("tablinq-instant"));
    assert_eq!(gateway_request.request_role.as_deref(), Some("branch"));
    assert_eq!(
        gateway_request.parent_request_id.as_deref(),
        Some("req-parent-1")
    );
    assert!(
        !gateway_request
            .client_id
            .as_deref()
            .unwrap_or_default()
            .is_empty()
    );
    assert!(
        !gateway_request
            .job_id
            .as_deref()
            .unwrap_or_default()
            .is_empty()
    );
    assert!(!gateway_request.streaming);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}
