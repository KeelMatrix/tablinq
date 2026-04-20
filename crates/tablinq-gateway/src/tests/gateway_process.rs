use super::*;

#[test]
fn gateway_health_and_readiness_endpoints_respond() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let (handle, snapshot) = spawn_gateway(config);

    let health = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        HEALTH_PATH,
        &[],
        &[],
        3_000,
    )
    .unwrap();
    assert_eq!(health.status_code, 200);

    let ready = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        READY_PATH,
        &[],
        &[],
        3_000,
    )
    .unwrap();
    assert_eq!(ready.status_code, 200);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn gateway_rejects_unauthorized_admin_requests() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        ADMIN_SHUTDOWN_PATH,
        &[],
        &[],
        3_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 401);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn gateway_propagates_request_ids_in_headers_and_logs() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let log_path = gateway_log_path(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        HEALTH_PATH,
        &[("x-request-id", "req-test-123")],
        &[],
        3_000,
    )
    .unwrap();
    assert_eq!(
        response.headers.get("x-request-id").map(String::as_str),
        Some("req-test-123")
    );

    let log = fs::read_to_string(log_path).unwrap();
    assert!(log.contains("req-test-123"));

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn gateway_status_and_stop_helpers_work_against_running_service() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let (handle, _snapshot) = spawn_gateway(config);

    let status = gateway_status(Some(&state_root)).unwrap();
    assert_eq!(status["running"], true);
    assert_eq!(status["ready"], true);

    let stop = stop_gateway_process(Some(&state_root)).unwrap();
    assert_eq!(stop["status"], "stopped");
    handle.join().unwrap().unwrap();
}

#[test]
fn gateway_doctor_helper_reports_scheduler_and_logs() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let (handle, _snapshot) = spawn_gateway(config);

    let doctor = gateway_doctor(Some(&state_root)).unwrap();
    assert_eq!(doctor["running"], true);
    assert_eq!(doctor["ready"], true);
    assert_eq!(doctor["scheduler"]["max_concurrent_upstream_jobs"], 1);
    assert!(
        doctor["logs"]["gateway"]
            .as_str()
            .unwrap()
            .ends_with("gateway.log")
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

#[test]
fn gateway_health_and_admin_follow_daemon_browser_health() {
    require_browser_integration_prereqs!();
    let _guard = integration_lock();
    let (_fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let daemon = spawn_daemon(config.clone());
    let endpoint = shared_protocol::build_state_paths(&state_root).ipc_endpoint;

    let opened = gptd::send_request(
        &endpoint,
        &shared_protocol::DaemonRequest::BrowserOpen(shared_protocol::BrowserOpenRequest {
            target_url: Some(fake_url.clone()),
            browser_path: None,
            headless: Some(true),
        }),
    )
    .unwrap();
    match opened {
        shared_protocol::DaemonResponse::Browser(snapshot) => {
            assert!(snapshot.running);
            assert!(snapshot.authenticated);
            assert!(snapshot.extension_connected);
        }
        other => panic!("unexpected browser open response: {other:?}"),
    }

    let (gateway_handle, snapshot) = spawn_gateway(config);
    let health = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        HEALTH_PATH,
        &[],
        &[],
        10_000,
    )
    .unwrap();
    let health_json: Value = serde_json::from_slice(&health.body).unwrap();
    assert_eq!(health_json["status"], "ok");
    assert_eq!(health_json["health"]["browser_connected"], true);
    assert_eq!(health_json["health"]["extension_connected"], true);
    assert_eq!(health_json["health"]["authenticated"], true);

    let admin = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        ADMIN_STATUS_PATH,
        &[("Authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    let admin_json: Value = serde_json::from_slice(&admin.body).unwrap();
    assert_eq!(admin_json["health"]["browser_connected"], true);
    assert_eq!(admin_json["health"]["extension_connected"], true);
    assert_eq!(admin_json["health"]["authenticated"], true);
    assert_eq!(admin_json["browser"]["running"], true);

    stop_gateway_process(Some(&state_root)).unwrap();
    gateway_handle.join().unwrap().unwrap();
    stop_daemon(&state_root, daemon);
}
