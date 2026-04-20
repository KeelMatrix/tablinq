use super::*;

#[test]
fn chat_completions_non_streaming_returns_openai_shape() {
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
        "messages": [
            { "role": "system", "content": "Answer briefly." },
            { "role": "user", "content": "hello from gateway" }
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
    assert_eq!(
        response.status_code,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["object"], "chat.completion");
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
fn sessions_endpoint_lists_recent_conversations() {
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
        "messages": [{ "role": "user", "content": "session listing baseline" }]
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
    let first_json: Value = serde_json::from_slice(&first.body).unwrap();
    let first_conversation_id = first_json["tablinq_conversation_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let second_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "tablinq_new_conversation": true,
        "messages": [{ "role": "user", "content": "session listing fresh conversation" }]
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
        ],
        &second_body,
        20_000,
    )
    .unwrap();
    let second_json: Value = serde_json::from_slice(&second.body).unwrap();
    let second_conversation_id = second_json["tablinq_conversation_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let sessions = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_SESSIONS_PATH,
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(sessions.status_code, 200);
    let sessions_json: Value = serde_json::from_slice(&sessions.body).unwrap();
    assert_eq!(sessions_json["object"], "tablinq.session_list");
    let session = &sessions_json["data"][0];
    assert_eq!(session["session_id"], "browser-primary");
    assert_eq!(session["latest_conversation_id"], second_conversation_id);
    let recent = session["recent_conversations"].as_array().unwrap();
    assert!(
        recent
            .iter()
            .any(|item| item["conversation_id"] == first_conversation_id)
    );
    assert!(
        recent
            .iter()
            .any(|item| item["conversation_id"] == second_conversation_id)
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn conversations_endpoint_lists_persisted_conversations() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let first = http_request(
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
            "messages": [{ "role": "user", "content": "conversation listing baseline" }]
        }))
        .unwrap(),
        20_000,
    )
    .unwrap();
    let first_json: Value = serde_json::from_slice(&first.body).unwrap();
    let first_conversation_id = first_json["tablinq_conversation_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let second = http_request(
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
            "tablinq_new_conversation": true,
            "messages": [{ "role": "user", "content": "conversation listing fresh" }]
        }))
        .unwrap(),
        20_000,
    )
    .unwrap();
    let second_json: Value = serde_json::from_slice(&second.body).unwrap();
    let second_conversation_id = second_json["tablinq_conversation_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let conversations = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_CONVERSATIONS_PATH,
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(conversations.status_code, 200);
    let conversations_json: Value = serde_json::from_slice(&conversations.body).unwrap();
    assert_eq!(conversations_json["object"], "tablinq.conversation_list");
    let items = conversations_json["data"].as_array().unwrap();
    assert_eq!(items[0]["conversation_id"], second_conversation_id);
    assert!(
        items
            .iter()
            .any(|item| item["conversation_id"] == first_conversation_id)
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn conversation_select_endpoint_switches_existing_and_fresh_conversations() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let first = http_request(
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
            "messages": [{ "role": "user", "content": "start conversation A" }]
        }))
        .unwrap(),
        20_000,
    )
    .unwrap();
    let first_json: Value = serde_json::from_slice(&first.body).unwrap();
    let first_conversation_id = first_json["tablinq_conversation_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let second = http_request(
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
            "tablinq_new_conversation": true,
            "messages": [{ "role": "user", "content": "start conversation B" }]
        }))
        .unwrap(),
        20_000,
    )
    .unwrap();
    let second_json: Value = serde_json::from_slice(&second.body).unwrap();
    let second_conversation_id = second_json["tablinq_conversation_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let switched = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CONVERSATIONS_SELECT_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        &serde_json::to_vec(&json!({ "conversation_id": first_conversation_id })).unwrap(),
        20_000,
    )
    .unwrap();
    assert_eq!(switched.status_code, 200);
    let switched_json: Value = serde_json::from_slice(&switched.body).unwrap();
    assert_eq!(switched_json["object"], "tablinq.conversation_selection");
    assert_eq!(
        switched_json["data"]["conversation_id"].as_str(),
        Some(first_conversation_id.as_str())
    );

    let continued = http_request(
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
            "messages": [{ "role": "user", "content": "continue the selected conversation" }]
        }))
        .unwrap(),
        20_000,
    )
    .unwrap();
    let continued_json: Value = serde_json::from_slice(&continued.body).unwrap();
    assert_eq!(
        continued_json["tablinq_conversation_id"].as_str(),
        Some(first_conversation_id.as_str())
    );

    let fresh_select = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CONVERSATIONS_SELECT_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        &serde_json::to_vec(&json!({ "new_conversation": true })).unwrap(),
        20_000,
    )
    .unwrap();
    assert_eq!(fresh_select.status_code, 200);
    let fresh_select_json: Value = serde_json::from_slice(&fresh_select.body).unwrap();
    assert!(fresh_select_json["data"]["conversation_id"].is_null());

    let fresh_after_select = http_request(
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
            "messages": [{ "role": "user", "content": "start conversation C after selecting new" }]
        }))
        .unwrap(),
        20_000,
    )
    .unwrap();
    let fresh_after_select_json: Value = serde_json::from_slice(&fresh_after_select.body).unwrap();
    let third_conversation_id = fresh_after_select_json["tablinq_conversation_id"]
        .as_str()
        .unwrap();
    assert_ne!(third_conversation_id, first_conversation_id);
    assert_ne!(third_conversation_id, second_conversation_id);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn conversation_select_endpoint_rejects_unknown_conversation_ids() {
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
        V1_CONVERSATIONS_SELECT_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        &serde_json::to_vec(&json!({ "conversation_id": "conv-missing" })).unwrap(),
        20_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 404);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["error"], "conversation_not_found");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_streaming_emits_sse_chunks_and_done() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let body = serde_json::to_vec(&json!({
        "model": "tablinq-search",
        "stream": true,
        "messages": [
            { "role": "user", "content": "latest roadmap" }
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
    assert_eq!(response.status_code, 200);
    assert_eq!(
        response.headers.get("content-type").map(String::as_str),
        Some("text/event-stream")
    );
    let text = String::from_utf8(response.body).unwrap();
    assert!(text.contains("chat.completion.chunk"));
    assert!(text.contains("[DONE]"));

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}
