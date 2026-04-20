use super::*;

#[test]
fn paperclip_like_queue_pressure_returns_retry_metadata() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.research_timeout_ms = Some(5_000);
        gateway.queue_timeout_ms = Some(250);
    }
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let first_body = paperclip_request_body("tablinq-research", "hold the session", false, &[]);
    let host = snapshot.host.clone();
    let port = snapshot.port;
    let auth_for_thread = auth.clone();
    let first = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-client-id", "paperclip"),
            ],
            &first_body,
            20_000,
        )
    });

    thread::sleep(Duration::from_millis(150));

    let second_body =
        paperclip_request_body("tablinq-instant", "time out while queued", false, &[]);
    let second = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-client-id", "paperclip"),
        ],
        &second_body,
        20_000,
    )
    .unwrap();
    assert_eq!(second.status_code, 429);
    let body: Value = serde_json::from_slice(&second.body).unwrap();
    assert_eq!(body["error"]["code"], "queue_timeout");
    assert_eq!(body["retry_after_ms"], 250);

    let first = first.join().unwrap().unwrap();
    assert_eq!(first.status_code, 200);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_run_queued_requests_in_fifo_order() {
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

    let first_body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "messages": [{ "role": "user", "content": "hold the slot long enough to queue" }]
    }))
    .unwrap();
    let host = snapshot.host.clone();
    let port = snapshot.port;
    let auth_for_thread = auth.clone();
    let first = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-first"),
            ],
            &first_body,
            20_000,
        )
        .unwrap()
    });

    thread::sleep(Duration::from_millis(200));

    let completions = Arc::new(Mutex::new(Vec::new()));
    let second_body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "messages": [
            { "role": "user", "content": "hello from second" }
        ]
    }))
    .unwrap();
    let second_host = snapshot.host.clone();
    let second_auth = auth.clone();
    let second_completions = Arc::clone(&completions);
    let second = thread::spawn(move || {
        let response = http_request(
            &second_host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", second_auth.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-second"),
            ],
            &second_body,
            20_000,
        )
        .unwrap();
        second_completions.lock().unwrap().push("second".to_owned());
        response
    });

    thread::sleep(Duration::from_millis(100));

    let third_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "messages": [
            { "role": "user", "content": "hello from third" }
        ]
    }))
    .unwrap();
    let third_host = snapshot.host.clone();
    let third_auth = auth.clone();
    let third_completions = Arc::clone(&completions);
    let third = thread::spawn(move || {
        let response = http_request(
            &third_host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", third_auth.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-third"),
            ],
            &third_body,
            20_000,
        )
        .unwrap();
        third_completions.lock().unwrap().push("third".to_owned());
        response
    });

    let cancel = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/requests/req-first/cancel",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(cancel.status_code, 200);

    let first = first.join().unwrap();
    assert_eq!(first.status_code, 409);
    let first_body: Value = serde_json::from_slice(&first.body).unwrap();
    assert_eq!(first_body["error"]["code"], "cancelled");
    let second = second.join().unwrap();
    let third = third.join().unwrap();
    assert_eq!(second.status_code, 200);
    assert_eq!(third.status_code, 200);
    assert_eq!(completions.lock().unwrap().as_slice(), ["second", "third"]);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_cancel_queued_request() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.research_timeout_ms = Some(5_000);
    }
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let first_body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "messages": [{ "role": "user", "content": "occupy the only slot" }]
    }))
    .unwrap();
    let host = snapshot.host.clone();
    let port = snapshot.port;
    let auth_for_thread = auth.clone();
    let first = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-running"),
            ],
            &first_body,
            20_000,
        )
    });

    thread::sleep(Duration::from_millis(250));

    let queued_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "messages": [{ "role": "user", "content": "cancel me while queued" }]
    }))
    .unwrap();
    let host = snapshot.host.clone();
    let auth_for_thread = auth.clone();
    let queued = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-cancel-me"),
            ],
            &queued_body,
            20_000,
        )
        .unwrap()
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            ADMIN_STATUS_PATH,
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        assert_eq!(status.status_code, 200);
        let status_body: Value = serde_json::from_slice(&status.body).unwrap();
        if status_body["scheduler"]["queued"].as_u64().unwrap_or(0) >= 1 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "queued request never reached the scheduler"
        );
        thread::sleep(Duration::from_millis(25));
    }

    let cancel = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/requests/req-cancel-me/cancel",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(cancel.status_code, 200);
    let cancel_body: Value = serde_json::from_slice(&cancel.body).unwrap();
    assert!(cancel_body["accepted"].as_bool().is_some());
    assert!(matches!(
        cancel_body["state"].as_str(),
        Some("queued" | "running")
    ));

    let queued = queued.join().unwrap();
    assert_eq!(queued.status_code, 409);
    let queued_body: Value = serde_json::from_slice(&queued.body).unwrap();
    assert_eq!(queued_body["error"]["code"], "cancelled");

    let cancel_running = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/requests/req-running/cancel",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(cancel_running.status_code, 200);

    let first = first.join().unwrap().unwrap();
    assert_eq!(first.status_code, 409);
    let first_body: Value = serde_json::from_slice(&first.body).unwrap();
    assert_eq!(first_body["error"]["code"], "cancelled");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_cancel_running_research_request() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.research_timeout_ms = Some(10_000);
    }
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "messages": [{ "role": "user", "content": "cancel a running research job" }]
    }))
    .unwrap();
    let host = snapshot.host.clone();
    let port = snapshot.port;
    let auth_for_thread = auth.clone();
    let running = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-running-cancel"),
            ],
            &body,
            20_000,
        )
        .unwrap()
    });

    thread::sleep(Duration::from_millis(500));

    let cancel = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/requests/req-running-cancel/cancel",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(cancel.status_code, 200);
    let cancel_body: Value = serde_json::from_slice(&cancel.body).unwrap();
    assert_eq!(cancel_body["accepted"], true);
    assert_eq!(cancel_body["state"], "running");

    let running = running.join().unwrap();
    assert_eq!(running.status_code, 409);
    let body: Value = serde_json::from_slice(&running.body).unwrap();
    assert_eq!(body["error"]["code"], "cancelled");

    let summary = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/requests/req-running-cancel",
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    assert_eq!(summary.status_code, 200);
    let summary_body: Value = serde_json::from_slice(&summary.body).unwrap();
    assert_eq!(summary_body["data"]["status"], "cancelled");
    assert_eq!(summary_body["data"]["cancelled"], true);
    assert!(summary_body["data"]["duration_ms"].as_u64().is_some());

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_cancel_running_streaming_ask_request() {
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
        "messages": [{ "role": "user", "content": "cancel a running ask stream" }]
    }))
    .unwrap();
    let host = snapshot.host.clone();
    let port = snapshot.port;
    let auth_for_thread = auth.clone();
    let running = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-stream-cancel"),
            ],
            &body,
            20_000,
        )
        .unwrap()
    });

    thread::sleep(Duration::from_millis(250));

    let cancel = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/requests/req-stream-cancel/cancel",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(cancel.status_code, 200);
    let cancel_body: Value = serde_json::from_slice(&cancel.body).unwrap();
    assert_eq!(cancel_body["accepted"], true);
    assert_eq!(cancel_body["state"], "running");
    assert!(
        cancel_body["message"]
            .as_str()
            .unwrap()
            .contains("cancellation requested")
    );

    let running = running.join().unwrap();
    assert_eq!(running.status_code, 200);
    let body = String::from_utf8(running.body).unwrap();
    assert!(body.contains("\"code\":\"cancelled\""));
    assert!(body.contains("[DONE]"));

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_cancel_during_reload_releases_the_queue_slot() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=reload"));
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.ask_timeout_ms = Some(10_000);
        gateway.queue_timeout_ms = Some(10_000);
    }
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let first_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "messages": [{ "role": "user", "content": "cancel me while the page reloads" }]
    }))
    .unwrap();
    let host = snapshot.host.clone();
    let port = snapshot.port;
    let auth_for_thread = auth.clone();
    let first = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-reload-cancel"),
            ],
            &first_body,
            20_000,
        )
        .unwrap()
    });

    thread::sleep(Duration::from_millis(250));

    let cancel = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/requests/req-reload-cancel/cancel",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(cancel.status_code, 200);

    let second_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "messages": [{ "role": "user", "content": "run after the cancelled reload request" }]
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
            ("x-request-id", "req-after-reload-cancel"),
        ],
        &second_body,
        20_000,
    )
    .unwrap();
    assert_eq!(second.status_code, 200);

    let first = first.join().unwrap();
    assert_eq!(first.status_code, 409);
    let first_body: Value = serde_json::from_slice(&first.body).unwrap();
    assert_eq!(first_body["error"]["code"], "cancelled");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn chat_completions_return_queue_timeout_when_wait_expires() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.research_timeout_ms = Some(5_000);
        gateway.queue_timeout_ms = Some(250);
    }
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let first_body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "messages": [{ "role": "user", "content": "occupy the only slot" }]
    }))
    .unwrap();
    let host = snapshot.host.clone();
    let port = snapshot.port;
    let auth_for_thread = auth.clone();
    let first = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
            ],
            &first_body,
            20_000,
        )
    });

    thread::sleep(Duration::from_millis(150));

    let second_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "messages": [{ "role": "user", "content": "time out in queue" }]
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
    assert_eq!(second.status_code, 429);
    let body: Value = serde_json::from_slice(&second.body).unwrap();
    assert_eq!(body["error"]["code"], "queue_timeout");

    let first = first.join().unwrap().unwrap();
    assert_eq!(first.status_code, 200);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn queue_reports_a_single_running_upstream_job_and_serializes_follow_up_work() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.max_concurrent_upstream_jobs = Some(1);
        gateway.queue_length = Some(4);
        gateway.queue_timeout_ms = Some(10_000);
    }
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let first_body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "messages": [{ "role": "user", "content": "occupy the only upstream slot" }]
    }))
    .unwrap();
    let host = snapshot.host.clone();
    let port = snapshot.port;
    let auth_for_thread = auth.clone();
    let first = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-single-slot-1"),
            ],
            &first_body,
            20_000,
        )
        .unwrap()
    });

    thread::sleep(Duration::from_millis(150));

    let second_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "messages": [{ "role": "user", "content": "wait behind the single slot" }]
    }))
    .unwrap();
    let host = snapshot.host.clone();
    let auth_for_thread = auth.clone();
    let second = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-single-slot-2"),
            ],
            &second_body,
            20_000,
        )
        .unwrap()
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            ADMIN_STATUS_PATH,
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        let status_body: Value = serde_json::from_slice(&status.body).unwrap();
        if status_body["scheduler"]["running"] == 1 && status_body["scheduler"]["queued"] == 1 {
            assert_eq!(status_body["scheduler"]["max_concurrent_upstream_jobs"], 1);
            break;
        }
        assert!(Instant::now() < deadline, "expected running=1 and queued=1");
        thread::sleep(Duration::from_millis(25));
    }

    let cancel = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/requests/req-single-slot-1/cancel",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(cancel.status_code, 200);

    let first = first.join().unwrap();
    assert_eq!(first.status_code, 409);
    let second = second.join().unwrap();
    assert_eq!(second.status_code, 200);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn queue_rejects_requests_when_per_client_limit_is_reached() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.per_client_limit = Some(1);
        gateway.queue_length = Some(4);
        gateway.queue_timeout_ms = Some(10_000);
    }
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let first_body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "messages": [{ "role": "user", "content": "occupy the per-client slot" }]
    }))
    .unwrap();
    let host = snapshot.host.clone();
    let port = snapshot.port;
    let auth_for_thread = auth.clone();
    let first = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-client-id", "shared-client"),
                ("x-request-id", "req-client-limit-1"),
            ],
            &first_body,
            20_000,
        )
        .unwrap()
    });

    thread::sleep(Duration::from_millis(150));

    let second_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "messages": [{ "role": "user", "content": "reject this second same-client request" }]
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
            ("x-client-id", "shared-client"),
        ],
        &second_body,
        20_000,
    )
    .unwrap();
    assert_eq!(second.status_code, 429);
    let second_body: Value = serde_json::from_slice(&second.body).unwrap();
    assert_eq!(second_body["error"]["code"], "client_limit_exceeded");
    assert!(second_body["retry_after_ms"].as_u64().is_some());

    let cancel = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/requests/req-client-limit-1/cancel",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(cancel.status_code, 200);

    let first = first.join().unwrap();
    assert_eq!(first.status_code, 409);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn queue_rejects_requests_when_the_queue_is_full() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.queue_length = Some(1);
        gateway.queue_timeout_ms = Some(10_000);
    }
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let first_body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "messages": [{ "role": "user", "content": "occupy the slot" }]
    }))
    .unwrap();
    let host = snapshot.host.clone();
    let port = snapshot.port;
    let auth_for_thread = auth.clone();
    let first = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-queue-full-1"),
            ],
            &first_body,
            20_000,
        )
        .unwrap()
    });

    thread::sleep(Duration::from_millis(150));

    let second_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "messages": [{ "role": "user", "content": "sit in the only queue slot" }]
    }))
    .unwrap();
    let host = snapshot.host.clone();
    let auth_for_thread = auth.clone();
    let second = thread::spawn(move || {
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-queue-full-2"),
            ],
            &second_body,
            20_000,
        )
        .unwrap()
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let status = http_request(
            &snapshot.host,
            snapshot.port,
            "GET",
            ADMIN_STATUS_PATH,
            &[("authorization", auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        let status_body: Value = serde_json::from_slice(&status.body).unwrap();
        if status_body["scheduler"]["queued"] == 1 {
            break;
        }
        assert!(Instant::now() < deadline, "expected a queued request");
        thread::sleep(Duration::from_millis(25));
    }

    let third_body = serde_json::to_vec(&json!({
        "model": "tablinq-instant",
        "messages": [{ "role": "user", "content": "overflow the queue" }]
    }))
    .unwrap();
    let third = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-client-id", "other-client"),
        ],
        &third_body,
        20_000,
    )
    .unwrap();
    assert_eq!(third.status_code, 429);
    let third_body: Value = serde_json::from_slice(&third.body).unwrap();
    assert_eq!(third_body["error"]["code"], "queue_full");
    assert!(third_body["retry_after_ms"].as_u64().is_some());

    let cancel = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/requests/req-queue-full-1/cancel",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(cancel.status_code, 200);

    let first = first.join().unwrap();
    assert_eq!(first.status_code, 409);
    let second = second.join().unwrap();
    assert_eq!(second.status_code, 200);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}
