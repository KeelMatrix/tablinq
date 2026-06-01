#![allow(unused_imports, dead_code)]
use super::*;

fn parse_sse_data_lines(response: &SimpleHttpResponse) -> Vec<String> {
    String::from_utf8(response.body.clone())
        .unwrap()
        .split("\n\n")
        .filter_map(|event| event.strip_prefix("data: "))
        .map(str::trim)
        .filter(|event| !event.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_sse_json_chunks(response: &SimpleHttpResponse) -> Vec<Value> {
    parse_sse_data_lines(response)
        .into_iter()
        .filter(|event| event != "[DONE]")
        .map(|event| serde_json::from_str(&event).unwrap())
        .collect()
}

#[test]
fn chat_completions_without_stream_option_falls_back_to_non_streaming_path() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    for index in 0..3 {
        let body = serde_json::to_vec(&json!({
            "model": "tablinq-instant",
            "messages": [
                { "role": "user", "content": format!("hello from gateway {index}") }
            ]
        }))
        .unwrap();
        let request_id = format!("req-valid-nonresearch-{index}");
        let response = http_request(
            &snapshot.host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", request_id.as_str()),
            ],
            &body,
            20_000,
        )
        .unwrap();
        assert_eq!(
            response.status_code,
            200,
            "{}",
            String::from_utf8_lossy(&response.body)
        );
        let body: Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(body["object"], "chat.completion");
        assert_eq!(body["request_id"], request_id);
        assert_eq!(body["model"], "tablinq-instant");
        assert_eq!(body["choices"][0]["message"]["role"], "assistant");
        assert!(body["tablinq_conversation_id"].as_str().is_some());
        assert_eq!(body["tablinq_session_id"], "browser-primary");
        assert!(
            body["choices"][0]["message"]["content"]
                .as_str()
                .unwrap()
                .contains("Fake response for")
        );
        assert_eq!(body["choices"][0]["finish_reason"], "stop");
        assert_eq!(body["usage"]["prompt_tokens"], Value::Null);
        assert_eq!(body["usage"]["completion_tokens"], Value::Null);
        assert_eq!(body["usage"]["total_tokens"], Value::Null);
        assert_eq!(body["tablinq_usage"]["status"], "unknown");
    }

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_with_explicit_stream_false_reaches_the_normal_path() {
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
        "stream": false,
        "messages": [
            { "role": "user", "content": "hello from explicit stream false" }
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
            ("x-request-id", "req-stream-false"),
        ],
        &body,
        20_000,
    )
    .unwrap();
    assert_eq!(
        response.status_code,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["request_id"], "req-stream-false");
    assert_eq!(body["model"], "tablinq-instant");
    assert_eq!(body["choices"][0]["message"]["role"], "assistant");
    assert_eq!(body["choices"][0]["finish_reason"], "stop");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_single_user_message_reaches_browser_without_role_labels() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let prompt = "system: keep this literal | assistant: keep this literal too";
    let body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "messages": [{ "role": "user", "content": prompt }]
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
    assert_eq!(
        response.status_code,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(
        body["choices"][0]["message"]["content"].as_str(),
        Some("Fake response for: system: keep this literal | assistant: keep this literal too")
    );
    let conversation_id = body["tablinq_conversation_id"].as_str().unwrap();

    let messages = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        &format!("/v1/conversations/{conversation_id}/messages"),
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(messages.status_code, 200);
    let messages_body: Value = serde_json::from_slice(&messages.body).unwrap();
    let items = messages_body["data"].as_array().unwrap();
    let user_message = items
        .iter()
        .find(|item| {
            item["source_kind"] == "request_detail"
                && item["role"] == "user"
                && item["content"] == prompt
        })
        .expect("raw user message in normalized conversation history");
    assert_eq!(user_message["content"], prompt);
    assert!(!items.iter().any(|item| {
        item["content"]
            .as_str()
            .is_some_and(|content| content.contains("USER:") || content.contains("SYSTEM:"))
    }));

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_support_fresh_and_existing_conversations() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let first_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "messages": [{ "role": "user", "content": "start the first conversation" }]
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
    assert_eq!(
        first.status_code,
        200,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    let first_json: Value = serde_json::from_slice(&first.body).unwrap();
    let first_conversation_id = first_json["tablinq_conversation_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(first_json["tablinq_session_id"], "browser-primary");

    let continue_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "tablinq_conversation_id": first_conversation_id,
        "messages": [{ "role": "user", "content": "continue the existing conversation" }]
    }))
    .unwrap();
    let continued = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        &continue_body,
        20_000,
    )
    .unwrap();
    assert_eq!(
        continued.status_code,
        200,
        "{}",
        String::from_utf8_lossy(&continued.body)
    );
    let continued_json: Value = serde_json::from_slice(&continued.body).unwrap();
    assert_eq!(
        continued_json["tablinq_conversation_id"].as_str(),
        Some(first_conversation_id.as_str())
    );
    assert_eq!(continued_json["tablinq_session_id"], "browser-primary");

    let fresh_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "tablinq_new_conversation": true,
        "messages": [{ "role": "user", "content": "start a fresh conversation" }]
    }))
    .unwrap();
    let fresh = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        &fresh_body,
        20_000,
    )
    .unwrap();
    assert_eq!(
        fresh.status_code,
        200,
        "{}",
        String::from_utf8_lossy(&fresh.body)
    );
    let fresh_json: Value = serde_json::from_slice(&fresh.body).unwrap();
    assert_ne!(
        fresh_json["tablinq_conversation_id"].as_str(),
        Some(first_conversation_id.as_str())
    );
    assert_eq!(fresh_json["tablinq_session_id"], "browser-primary");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_reject_unknown_conversation_ids() {
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
        "tablinq_conversation_id": "conv-missing",
        "messages": [{ "role": "user", "content": "fail with a missing conversation" }]
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
    assert_eq!(response.status_code, 404);
    let json: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(json["error"]["code"], "conversation_not_found");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_stream_true_returns_final_only_sse() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let prompt = "final-only stream answer";
    let body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "stream": true,
        "messages": [
            { "role": "user", "content": prompt }
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
            ("x-request-id", "req-stream-final-only"),
        ],
        &body,
        20_000,
    )
    .unwrap();
    assert_eq!(
        response.status_code,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(
        response.headers.get("content-type").map(String::as_str),
        Some("text/event-stream")
    );

    let data_lines = parse_sse_data_lines(&response);
    assert_eq!(data_lines.last().map(String::as_str), Some("[DONE]"));
    let chunks = parse_sse_json_chunks(&response);
    assert_eq!(chunks.len(), 2);
    let final_text = format!("Fake response for: {prompt}");
    assert_eq!(chunks[0]["object"], "chat.completion.chunk");
    assert!(chunks[0]["id"].as_str().unwrap().starts_with("chatcmpl-"));
    assert_eq!(chunks[0]["model"], "tablinq-instant");
    assert_eq!(chunks[0]["request_id"], "req-stream-final-only");
    assert!(chunks[0]["tablinq_job_id"].as_str().is_some());
    assert!(chunks[0]["tablinq_conversation_id"].as_str().is_some());
    assert_eq!(chunks[0]["tablinq_session_id"], "browser-primary");
    assert_eq!(chunks[0]["choices"][0]["index"], 0);
    assert_eq!(
        chunks[0]["choices"][0]["delta"]["content"].as_str(),
        Some(final_text.as_str())
    );
    assert_eq!(chunks[0]["choices"][0]["finish_reason"], Value::Null);
    assert_eq!(chunks[1]["object"], "chat.completion.chunk");
    assert_eq!(chunks[1]["id"], chunks[0]["id"]);
    assert_eq!(chunks[1]["choices"][0]["delta"], json!({}));
    assert_eq!(chunks[1]["choices"][0]["finish_reason"], "stop");
    assert!(!chunks.iter().any(|chunk| {
        chunk["choices"][0]["delta"]["content"]
            .as_str()
            .is_some_and(|content| content != final_text)
    }));

    let content_chunks = chunks
        .iter()
        .filter(|chunk| chunk["choices"][0]["delta"]["content"].as_str().is_some())
        .count();
    assert_eq!(content_chunks, 1);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_stream_true_submits_same_browser_prompt_as_stream_false() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let prompt = "system: keep literal | assistant: keep literal | stream parity";
    let non_stream_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "stream": false,
        "messages": [{ "role": "user", "content": prompt }]
    }))
    .unwrap();
    let non_stream = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-request-id", "req-stream-parity-false"),
        ],
        &non_stream_body,
        20_000,
    )
    .unwrap();
    assert_eq!(non_stream.status_code, 200);
    let non_stream_json: Value = serde_json::from_slice(&non_stream.body).unwrap();

    let stream_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "stream": true,
        "messages": [{ "role": "user", "content": prompt }]
    }))
    .unwrap();
    let stream = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-request-id", "req-stream-parity-true"),
        ],
        &stream_body,
        20_000,
    )
    .unwrap();
    assert_eq!(stream.status_code, 200);
    let stream_chunks = parse_sse_json_chunks(&stream);
    let final_text = format!("Fake response for: {prompt}");
    assert_eq!(
        non_stream_json["choices"][0]["message"]["content"].as_str(),
        Some(final_text.as_str())
    );
    assert_eq!(
        stream_chunks[0]["choices"][0]["delta"]["content"].as_str(),
        Some(final_text.as_str())
    );

    for conversation_id in [
        non_stream_json["tablinq_conversation_id"].as_str().unwrap(),
        stream_chunks[0]["tablinq_conversation_id"]
            .as_str()
            .unwrap(),
    ] {
        let messages = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            &format!("/v1/conversations/{conversation_id}/messages"),
            &[("authorization", auth.as_str())],
            &[],
            20_000,
        )
        .unwrap();
        assert_eq!(messages.status_code, 200);
        let messages_body: Value = serde_json::from_slice(&messages.body).unwrap();
        assert!(
            messages_body["data"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| {
                    item["source_kind"] == "request_detail"
                        && item["role"] == "user"
                        && item["content"] == prompt
                })
        );
    }

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_stream_true_execution_errors_are_sse_errors() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?missing=search"));
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let body = serde_json::to_vec(&json!({
        "model": "tablinq-thinking-standard",
        "tablinq_mode": "search",
        "stream": true,
        "messages": [
            { "role": "user", "content": "search should be unavailable" }
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
            ("x-request-id", "req-stream-capability"),
        ],
        &body,
        20_000,
    )
    .unwrap();
    assert_eq!(
        response.status_code,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    assert_eq!(
        response.headers.get("content-type").map(String::as_str),
        Some("text/event-stream")
    );
    let data_lines = parse_sse_data_lines(&response);
    assert_eq!(data_lines.last().map(String::as_str), Some("[DONE]"));
    let error_event: Value = serde_json::from_str(&data_lines[0]).unwrap();
    assert_eq!(error_event["object"], "error");
    assert_eq!(error_event["request_id"], "req-stream-capability");
    assert_eq!(error_event["error"]["type"], "invalid_request_error");
    assert_eq!(error_event["error"]["code"], "capability_missing");
    assert!(
        error_event["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("capability"))
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn openclaude_shaped_stream_true_request_returns_final_only_sse() {
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
        "store": false,
        "temperature": 0.2,
        "top_p": 0.9,
        "max_tokens": 128,
        "max_completion_tokens": 64,
        "metadata": { "client": "openclaude-regression" },
        "messages": [{ "role": "user", "content": "openclaude streaming regression" }]
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
            ("x-request-id", "req-openclaude-stream"),
        ],
        &body,
        20_000,
    )
    .unwrap();
    assert_eq!(
        response.status_code,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let chunks = parse_sse_json_chunks(&response);
    assert_eq!(chunks.len(), 2);
    for chunk in &chunks {
        assert_eq!(chunk["object"], "chat.completion.chunk");
        assert_eq!(chunk["model"], "tablinq-instant");
    }
    assert_eq!(
        chunks[0]["choices"][0]["delta"]["content"].as_str(),
        Some("Fake response for: openclaude streaming regression")
    );
    assert_eq!(chunks[1]["choices"][0]["finish_reason"], "stop");
    assert_eq!(parse_sse_data_lines(&response).last().unwrap(), "[DONE]");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}
