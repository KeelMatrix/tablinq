#![allow(unused_imports, dead_code)]
use super::*;

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
    assert_eq!(items[0]["tablinq_conversation_id"], second_conversation_id);
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
    assert_eq!(
        switched_json["data"]["tablinq_conversation_id"].as_str(),
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
    assert!(body["error"].is_object());
    assert_eq!(body["error"]["type"], "not_found_error");
    assert_eq!(body["error"]["code"], "conversation_not_found");
    assert_eq!(body["error"]["param"], Value::Null);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn conversation_select_endpoint_rejects_conflicting_targets_before_lookup() {
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
        &serde_json::to_vec(&json!({
            "conversation_id": "conv-missing",
            "new_conversation": true
        }))
        .unwrap(),
        20_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 400);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert!(body["error"].is_object());
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["code"], "invalid_request");
    assert_eq!(body["error"]["param"], Value::Null);
    assert!(
        body["error"]["message"]
            .as_str()
            .is_some_and(|text| text.contains("cannot be combined"))
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn conversation_select_endpoint_rejects_invalid_json_with_nested_error() {
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
        br#"{"conversation_id":"conv-1""#,
        20_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 400);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert!(body["request_id"].as_str().is_some());
    assert!(body["error"].is_object());
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(body["error"]["code"], "invalid_request_json");
    assert_eq!(body["error"]["param"], Value::Null);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}
