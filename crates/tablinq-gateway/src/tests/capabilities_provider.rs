use super::*;

fn assert_no_provider_role_artifacts(body: &Value) {
    let body_text = body.to_string().to_ascii_lowercase();
    for marker in [
        "accepted_message_roles",
        "role_handling",
        "request_annotations",
        "tablinq_request_role",
        "tablinq_parent_request_id",
        "only user messages are accepted",
        "non-user roles",
        "user role",
        "\"role\"",
        "\"roles\"",
        "system",
        "developer",
        "assistant",
        "tool",
    ] {
        assert!(
            !body_text.contains(marker),
            "unexpected marker {marker} in {body_text}"
        );
    }
}

#[test]
fn provider_endpoint_exposes_openclaude_preset_metadata() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.ask_timeout_ms = Some(30_000);
    }
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_PROVIDER_PATH,
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(
        response.status_code,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_no_provider_role_artifacts(&body);
    assert_eq!(body["object"], "tablinq.provider");
    assert_eq!(body["capability_source"], "live");
    assert_eq!(
        body["openai_compatibility"]["chat_completions"]["input_handling"][0],
        "chat_completions input is reduced to user-authored text content before browser submission"
    );
    assert_eq!(
        body["provider_capabilities"]["capabilities"]["search"]["snapshot_status"],
        "observed"
    );
    assert_eq!(
        body["provider_capabilities"]["capabilities"]["research"]["snapshot_status"],
        "observed"
    );
    assert_eq!(
        body["provider_capabilities"]["capabilities"]["uploads"]["snapshot_status"],
        "observed"
    );
    assert_eq!(
        body["provider_capabilities"]["capabilities"]["artifacts"]["snapshot_status"],
        "observed"
    );
    assert_eq!(
        body["provider_capabilities"]["max_safe_parallelism"]["requests"],
        1
    );
    assert_eq!(
        body["provider_capabilities"]["subagent_recommendation"]["policy"],
        "queue"
    );
    assert_eq!(body["presets"]["openclaude"]["max_parallel_requests"], 1);
    assert_eq!(body["presets"]["openclaude"]["subagents"], "queued");
    assert_eq!(
        body["presets"]["openclaude"]["recommended_presets"],
        json!([
            "tablinq-instant",
            "tablinq-thinking-standard",
            "tablinq-search",
        ])
    );
    assert_eq!(body["presets"]["openclaude"]["streaming"], false);
    assert!(
        body["presets"]["openclaude"]["streaming_note"]
            .as_str()
            .is_some_and(|note| note.contains("stream to false or omit it"))
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn provider_endpoint_exposes_paperclip_preset_metadata() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_PROVIDER_PATH,
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(
        response.status_code,
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    let body_text = String::from_utf8(response.body.clone()).unwrap();
    assert_no_provider_role_artifacts(&body);
    assert_eq!(
        body["presets"]["paperclip"]["transport"],
        "openai_compatible_http"
    );
    assert_eq!(body["openai_compatibility"]["mode"], "subset");
    assert_eq!(body["request_queueing"]["parallel_work"], "queue");
    assert_eq!(
        body["provider_capabilities"]["subagent_recommendation"]["policy"],
        "queue"
    );
    assert_eq!(body["presets"]["paperclip"]["max_parallel_requests"], 1);
    assert_eq!(body["presets"]["paperclip"]["streaming"], false);
    assert_eq!(body["presets"]["paperclip"]["files"], true);
    assert_eq!(body["presets"]["paperclip"]["artifacts"], true);
    assert_eq!(
        body["presets"]["paperclip"]["subagent_recommendation"],
        "queue"
    );
    assert_eq!(body["presets"]["paperclip"]["usage_accounting"], "unknown");
    assert_eq!(
        body["presets"]["paperclip"]["recommended_presets"],
        json!(["tablinq-instant", "tablinq-search", "tablinq-research"])
    );
    assert!(!body_text.to_ascii_lowercase().contains("managed"));

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn documented_provider_contract_surfaces_are_present() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let provider = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_PROVIDER_PATH,
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(provider.status_code, 200);
    let provider_body: Value = serde_json::from_slice(&provider.body).unwrap();
    assert_no_provider_role_artifacts(&provider_body);
    assert_eq!(provider_body["openai_compatibility"]["mode"], "subset");
    assert_eq!(
        provider_body["openai_compatibility"]["supported_endpoints"],
        json!([
            "/v1/chat/completions",
            "/v1/models",
            "/v1/provider",
            "/v1/capabilities",
            "/v1/files",
            "/v1/artifacts",
            "/v1/requests",
        ])
    );
    assert_eq!(
        provider_body["openai_compatibility"]["chat_completions"]["input_handling"][1],
        "browser submission preserves message text exactly and does not add transcript wrappers"
    );
    assert!(
        provider_body["openai_compatibility"]["chat_completions"]["unsupported_parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "live streaming responses")
    );
    assert!(
        provider_body["openai_compatibility"]["chat_completions"]["constraints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "stream must be false or omitted")
    );
    assert_eq!(provider_body["request_queueing"]["dependent_work"], "queue");
    assert_eq!(
        provider_body["provider_capabilities"]["capabilities"]["streaming"]["gateway_support"],
        "unsupported"
    );
    assert_eq!(
        provider_body["presets"]["openclaude"]["max_parallel_requests"],
        1
    );
    assert_eq!(
        provider_body["presets"]["openclaude"]["subagents"],
        "queued"
    );
    assert_eq!(
        provider_body["presets"]["paperclip"]["max_parallel_requests"],
        1
    );
    assert_eq!(
        provider_body["presets"]["paperclip"]["subagent_recommendation"],
        "queue"
    );
    assert_eq!(
        provider_body["preset_ids"],
        json!([
            "tablinq-instant",
            "tablinq-thinking-standard",
            "tablinq-thinking-extended",
            "tablinq-search",
            "tablinq-research",
        ])
    );
    assert!(
        !String::from_utf8(provider.body.clone())
            .unwrap()
            .to_ascii_lowercase()
            .contains("managed")
    );

    let chat_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "tablinq_new_conversation": true,
        "messages": [
            { "role": "user", "content": "document the provider contract" }
        ]
    }))
    .unwrap();
    let chat = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        &chat_body,
        20_000,
    )
    .unwrap();
    assert_eq!(chat.status_code, 200);
    let chat_body: Value = serde_json::from_slice(&chat.body).unwrap();
    assert!(chat_body["request_id"].as_str().is_some());
    assert!(chat_body["tablinq_conversation_id"].as_str().is_some());
    assert_eq!(chat_body["tablinq_session_id"], "browser-primary");

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
    assert!(
        usage_body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.as_str()
                == Some("token usage is unavailable for the browser-backed provider"))
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn openclaude_primary_streaming_requests_are_rejected() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let body = openclaude_request_body("tablinq-search", "latest roadmap", "primary", true, None);
    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-request-id", "oc-primary-1"),
        ],
        &body,
        20_000,
    )
    .unwrap();
    assert_eq!(
        response.status_code,
        400,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["request_id"], "oc-primary-1");
    assert_eq!(body["error"]["code"], "streaming_unsupported");
    assert_eq!(body["error"]["type"], "invalid_request_error");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn openclaude_branch_requests_are_serialized_behind_primary_work() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.research_timeout_ms = Some(15_000);
        gateway.queue_timeout_ms = Some(15_000);
    }
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let primary_body = openclaude_request_body(
        "tablinq-research",
        "hold the browser session",
        "primary",
        false,
        None,
    );
    let host = snapshot.host.clone();
    let port = snapshot.port;
    let auth_for_thread = auth.clone();
    let primary = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "oc-primary-1"),
            ],
            &primary_body,
            20_000,
        )
        .unwrap()
    });

    thread::sleep(Duration::from_millis(250));

    let branch_body = openclaude_request_body(
        "tablinq-instant",
        "branch after primary",
        "branch",
        false,
        Some("oc-primary-1"),
    );
    let host = snapshot.host.clone();
    let auth_for_thread = auth.clone();
    let branch = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "oc-branch-1"),
            ],
            &branch_body,
            20_000,
        )
        .unwrap()
    });

    thread::sleep(Duration::from_millis(150));

    let cancel = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/requests/oc-primary-1/cancel",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(cancel.status_code, 200);

    let primary = primary.join().unwrap();
    assert_eq!(primary.status_code, 409);
    let branch = branch.join().unwrap();
    assert_eq!(branch.status_code, 200);
    let branch_body: Value = serde_json::from_slice(&branch.body).unwrap();
    assert_eq!(branch_body["tablinq_request_role"], "branch");
    assert_eq!(branch_body["tablinq_parent_request_id"], "oc-primary-1");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn models_endpoint_returns_openai_model_catalog() {
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
        "GET",
        V1_MODELS_PATH,
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_no_provider_role_artifacts(&body);
    assert_eq!(body["object"], "list");
    assert_eq!(body["tablinq_capability_source"], "live");
    assert_eq!(
        body["tablinq_preset_ids"],
        json!([
            "tablinq-instant",
            "tablinq-thinking-standard",
            "tablinq-thinking-extended",
            "tablinq-search",
            "tablinq-research",
        ])
    );
    let models = body["data"].as_array().unwrap();
    assert_eq!(models.len(), 5);
    assert_eq!(models[0]["id"], "tablinq-instant");
    assert_eq!(models[0]["tablinq_model_class"], "instant");
    assert_eq!(models[0]["tablinq_effort"], "none");
    assert_eq!(models[0]["tablinq_streaming"], false);
    assert_eq!(models[0]["tablinq_streaming_status"], "unsupported");
    assert_eq!(models[1]["id"], "tablinq-thinking-standard");
    assert_eq!(models[1]["tablinq_model_class"], "thinking");
    assert_eq!(models[1]["tablinq_effort"], "standard");
    assert_eq!(models[2]["id"], "tablinq-thinking-extended");
    assert_eq!(models[2]["tablinq_effort"], "extended");
    assert_eq!(models[3]["id"], "tablinq-search");
    assert_eq!(models[3]["tablinq_preset_kind"], "composer_mode");
    assert_eq!(models[3]["tablinq_mode"], "search");
    assert_eq!(models[4]["id"], "tablinq-research");
    assert_eq!(models[4]["tablinq_preset_kind"], "research_flow");
    assert_eq!(models[4]["tablinq_execution_kind"], "research");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn capabilities_endpoint_reports_alias_remaps_for_stable_models() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?variant=aria"));
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_CAPABILITIES_PATH,
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_no_provider_role_artifacts(&body);
    assert_eq!(body["object"], "tablinq.capability_catalog");
    assert_eq!(body["source"], "live");
    assert_eq!(
        body["provider"]["observed_preset_ids"],
        json!([
            "tablinq-instant",
            "tablinq-thinking-standard",
            "tablinq-thinking-extended",
            "tablinq-search",
            "tablinq-research",
        ])
    );
    assert_eq!(body["observed_ui"]["search_mode"]["status"], "observed");
    assert!(body["presets"].as_array().unwrap().iter().any(|item| {
        item["id"] == "tablinq-thinking-standard"
            && item["tablinq_aliases"]["model_classes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|alias| alias.as_str().is_some())
    }));

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn capabilities_endpoint_marks_missing_features_unavailable() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?missing=effort,search,research"));
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_CAPABILITIES_PATH,
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    let search = body["presets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "tablinq-search")
        .unwrap();
    let thinking = body["presets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "tablinq-thinking-standard")
        .unwrap();
    let research = body["presets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "tablinq-research")
        .unwrap();
    assert_eq!(thinking["tablinq_snapshot_status"], "not_observed");
    assert_eq!(search["tablinq_snapshot_status"], "not_observed");
    assert_eq!(research["tablinq_snapshot_status"], "not_observed");
    assert!(
        !search["tablinq_snapshot_note"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        body["provider"]["capabilities"]["search"]["snapshot_status"],
        "not_observed"
    );
    assert_eq!(
        body["provider"]["capabilities"]["research"]["snapshot_status"],
        "not_observed"
    );
    let observed_ids = body["provider"]["observed_preset_ids"].as_array().unwrap();
    assert_eq!(body["provider"]["preset_ids"].as_array().unwrap().len(), 5);
    assert_eq!(observed_ids.len(), 1);
    assert_eq!(observed_ids[0], "tablinq-instant");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn capability_cache_returns_cached_catalog_for_profile_workspace_key() {
    let _guard = integration_lock();
    let dir = tempdir().unwrap();
    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let cache = CapabilityCatalogCache::load(&config).unwrap();
    let report = CapabilityReport {
        reported_at: Utc::now(),
        selector_pack_id: "chatgpt-default".to_owned(),
        selector_pack_version: "v1".to_owned(),
        matrix: shared_protocol::CapabilityMatrix {
            composer_ready: true,
            submit_available: true,
            model_picker_visible: true,
            thinking_effort_visible: true,
            search_mode_visible: true,
            research_mode_visible: true,
            upload_control_visible: true,
            downloadable_artifacts_observable: true,
        },
        aliases: shared_protocol::RuntimeAliases {
            modes: BTreeMap::from([
                ("normal".to_owned(), vec!["Chat".to_owned()]),
                ("search".to_owned(), vec!["Browse the web".to_owned()]),
                ("research".to_owned(), vec!["Deep research".to_owned()]),
            ]),
            model_classes: BTreeMap::from([
                ("instant".to_owned(), vec!["Instant answers".to_owned()]),
                ("thinking".to_owned(), vec!["Deep thinking".to_owned()]),
            ]),
            efforts: BTreeMap::from([
                ("standard".to_owned(), vec!["Standard reasoning".to_owned()]),
                ("extended".to_owned(), vec!["Extended reasoning".to_owned()]),
            ]),
        },
        observed_ui: shared_protocol::CapabilityObservedUi::default(),
    };

    cache
        .remember(
            "profile-a::workspace-a",
            "profile-a",
            "workspace-a",
            &report,
        )
        .unwrap();
    let cached = cache.cached("profile-a::workspace-a").unwrap().unwrap();
    assert_eq!(cached.profile_dir, "profile-a");
    assert_eq!(cached.workspace_key, "workspace-a");
    assert_eq!(
        cached.report.aliases.model_classes["thinking"],
        vec!["Deep thinking".to_owned()]
    );
}
