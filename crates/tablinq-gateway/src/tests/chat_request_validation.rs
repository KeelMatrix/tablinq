use super::*;

#[test]
fn unknown_model_returns_openai_shaped_invalid_request_error() {
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
            ("x-request-id", "req-unknown-model"),
        ],
        &serde_json::to_vec(&json!({
            "model": "tablinq-missing",
            "messages": [{ "role": "user", "content": "hello" }]
        }))
        .unwrap(),
        5_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 400);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["request_id"], "req-unknown-model");
    assert_eq!(body["error"]["type"], "invalid_request_error");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn stream_true_returns_openai_shaped_invalid_request_error() {
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
            ("x-request-id", "req-stream-unsupported"),
        ],
        &serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "stream": true,
            "messages": [{ "role": "user", "content": "hello" }]
        }))
        .unwrap(),
        5_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 400);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["request_id"], "req-stream-unsupported");
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["code"], "streaming_unsupported");
    assert!(body["error"]["message"].as_str().is_some_and(|message| {
        message.contains("append-only stable") && message.contains("stream:false")
    }));

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn common_ignored_openai_fields_do_not_fail_chat_requests() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
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
        ],
        &serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "hello with ignored fields" }],
            "temperature": 0.2,
            "top_p": 0.9,
            "max_tokens": 128,
            "max_completion_tokens": 64,
            "stop": ["END"],
            "presence_penalty": 0.0,
            "frequency_penalty": 0.0,
            "user": "client-user",
            "metadata": { "client": "integration-test" },
            "logprobs": true,
            "stream_options": { "include_usage": true },
            "tool_choice": "auto",
            "parallel_tool_calls": true
        }))
        .unwrap(),
        20_000,
    )
    .unwrap();
    assert_eq!(
        response.status_code,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn non_user_roles_return_invalid_request_errors() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    for (request_id, role) in [
        ("req-role-system", "system"),
        ("req-role-developer", "developer"),
        ("req-role-assistant", "assistant"),
        ("req-role-tool", "tool"),
        ("req-role-custom", "custom_role"),
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
                "messages": [{ "role": role, "content": "hello" }]
            }))
            .unwrap(),
            5_000,
        )
        .unwrap();
        assert_eq!(
            response.status_code,
            400,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["request_id"], request_id);
        assert_eq!(body["error"]["type"], "invalid_request_error");
        assert_eq!(body["error"]["code"], "unsupported_role");
    }

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn message_tool_calls_return_invalid_request_errors() {
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
            ("x-request-id", "req-tool-calls"),
        ],
        &serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{
                "role": "user",
                "content": "hello",
                "tool_calls": [{ "id": "call-1", "type": "function" }]
            }]
        }))
        .unwrap(),
        5_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 400);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["request_id"], "req-tool-calls");
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["code"], "unsupported_tool_calls");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn unsupported_tools_response_format_and_n_return_400() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    for (request_id, body) in [
        (
            "req-tools",
            json!({
                "model": "tablinq-instant",
                "messages": [{ "role": "user", "content": "hello" }],
                "tools": [{ "type": "function", "function": { "name": "demo" } }]
            }),
        ),
        (
            "req-response-format",
            json!({
                "model": "tablinq-instant",
                "messages": [{ "role": "user", "content": "hello" }],
                "response_format": { "type": "json_schema" }
            }),
        ),
        (
            "req-n",
            json!({
                "model": "tablinq-instant",
                "messages": [{ "role": "user", "content": "hello" }],
                "n": 2
            }),
        ),
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
            &serde_json::to_vec(&body).unwrap(),
            5_000,
        )
        .unwrap();
        assert_eq!(
            response.status_code,
            400,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        let json: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(json["request_id"], request_id);
        assert_eq!(json["error"]["type"], "invalid_request_error");
    }

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn invalid_json_and_unknown_fields_do_not_become_transport_errors() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let invalid_json = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-request-id", "req-invalid-json"),
        ],
        b"{",
        5_000,
    )
    .unwrap();
    assert_eq!(invalid_json.status_code, 400);
    let invalid_json_body: Value = serde_json::from_slice(&invalid_json.body).unwrap();
    assert_eq!(invalid_json_body["request_id"], "req-invalid-json");
    assert_eq!(invalid_json_body["error"]["type"], "invalid_request_error");
    assert_ne!(invalid_json_body["error"]["code"], "transport_error");

    let unknown_field = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-request-id", "req-unknown-field"),
        ],
        &serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "hello" }],
            "unexpected_field": true
        }))
        .unwrap(),
        5_000,
    )
    .unwrap();
    assert_eq!(unknown_field.status_code, 400);
    let unknown_field_body: Value = serde_json::from_slice(&unknown_field.body).unwrap();
    assert_eq!(unknown_field_body["request_id"], "req-unknown-field");
    assert_eq!(unknown_field_body["error"]["type"], "invalid_request_error");
    assert_ne!(unknown_field_body["error"]["code"], "transport_error");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn blank_staged_file_ids_return_400_invalid_request_errors() {
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
            ("x-request-id", "req-blank-file-id"),
        ],
        &serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "tablinq_file_ids": ["   "],
            "messages": [{ "role": "user", "content": "hello" }]
        }))
        .unwrap(),
        5_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 400);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["request_id"], "req-blank-file-id");
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_ne!(body["error"]["code"], "transport_error");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}
