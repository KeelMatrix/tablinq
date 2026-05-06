use super::*;

#[test]
fn gateway_health_and_readiness_endpoints_respond() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let health = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        HEALTH_PATH,
        &[("authorization", auth.as_str())],
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
        &[("authorization", auth.as_str())],
        &[],
        3_000,
    )
    .unwrap();
    assert_eq!(ready.status_code, 200);
}

#[test]
fn gateway_health_and_readiness_endpoints_require_authorization() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

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
    assert_eq!(health.status_code, 401);

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
    assert_eq!(ready.status_code, 401);

    let bad_health = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        HEALTH_PATH,
        &[("authorization", "Bearer wrong-token")],
        &[],
        3_000,
    )
    .unwrap();
    assert_eq!(bad_health.status_code, 401);
}

#[test]
fn gateway_rejects_unauthorized_admin_requests() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

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
}

#[test]
fn gateway_startup_fails_when_gateway_api_key_is_blank() {
    let _guard = integration_lock();
    let (_dir, mut config) = test_config();
    config.file.gateway.as_mut().unwrap().api_key = Some("   ".to_owned());

    let err = run_gateway(config).unwrap_err();
    assert!(format!("{err:#}").contains("gateway.api_key must be configured"));
}

#[test]
fn gateway_admin_clear_removes_database_state_and_artifact_files() {
    let _guard = integration_lock();
    let (dir, config) = test_config();
    let artifact_path = dir.path().join("artifact.txt");
    fs::write(&artifact_path, "artifact").unwrap();
    seed_gateway_clear_fixture(&config, artifact_path.display().to_string());
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config.clone());
    let snapshot = &gateway.snapshot;

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        crate::routes::ADMIN_CLEAR_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        br#"{"all":true,"force":true}"#,
        3_000,
    )
    .unwrap();

    assert_eq!(response.status_code, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["cleared"], true);
    assert_eq!(body["result"]["sessions_deleted"], 1);
    assert_eq!(body["result"]["conversations_deleted"], 1);
    assert_eq!(body["result"]["jobs_deleted"], 1);
    assert_eq!(body["result"]["artifacts_deleted"], 1);
    assert_eq!(body["result"]["artifact_files_deleted"], 1);
    assert!(!artifact_path.exists());
    let db = Database::open(&config.paths.db_path).unwrap();
    assert!(db.list_sessions().unwrap().sessions.is_empty());
    assert!(db.list_conversations().unwrap().conversations.is_empty());
    assert!(db.list_jobs().unwrap().jobs.is_empty());
    assert!(db.list_artifacts().unwrap().artifacts.is_empty());
}

#[test]
fn gateway_propagates_request_ids_in_headers_and_logs() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let log_path = gateway_log_path(&config);
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        HEALTH_PATH,
        &[
            ("authorization", auth.as_str()),
            ("x-request-id", "req-test-123"),
        ],
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
fn health_request_reports_attached_after_api_attach() {
    require_browser_integration_prereqs!();
    let _guard = integration_lock();
    let (_fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url.clone());
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let attach = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/browser/attach",
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        br#"{}"#,
        20_000,
    )
    .unwrap();
    let attach_json: Value = serde_json::from_slice(&attach.body).unwrap();
    assert_eq!(attach_json["data"]["running"], true);
    assert_eq!(attach_json["data"]["target_tab_found"], true);
    assert_eq!(attach_json["data"]["authenticated"], true);
    assert_eq!(attach_json["data"]["extension_connected"], true);
    assert_eq!(attach_json["data"]["page_ready"], true);

    let health = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        HEALTH_PATH,
        &[("authorization", auth.as_str())],
        &[],
        10_000,
    )
    .unwrap();
    let health_json: Value = serde_json::from_slice(&health.body).unwrap();
    assert_eq!(health_json["status"], "ok");
    assert_eq!(health_json["health"]["browser_connected"], true);
    assert_eq!(health_json["health"]["extension_connected"], true);
    assert_eq!(health_json["health"]["authenticated"], true);
    assert!(health_json["daemon_health"].is_null());
}

#[test]
fn admin_status_health_and_browser_are_consistent_after_api_attach() {
    require_browser_integration_prereqs!();
    let _guard = integration_lock();
    let (_fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let attach = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/browser/attach",
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        br#"{}"#,
        20_000,
    )
    .unwrap();
    assert_eq!(attach.status_code, 200);

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
    assert_eq!(admin_json["browser"]["target_tab_found"], true);
    assert_eq!(admin_json["browser"]["authenticated"], true);
    assert_eq!(admin_json["browser"]["extension_connected"], true);
    assert!(admin_json["daemon_health"].is_null());
}

#[test]
fn health_request_still_reports_degraded_when_gateway_core_browser_is_detached() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let auth = auth_header(&config);
    let gateway = RunningGateway::start(config);
    let snapshot = &gateway.snapshot;

    let health = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        HEALTH_PATH,
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    let health_json: Value = serde_json::from_slice(&health.body).unwrap();
    assert_eq!(health_json["status"], "degraded");
    assert_eq!(health_json["health"]["browser_connected"], false);
    assert_eq!(health_json["health"]["extension_connected"], false);
    assert_eq!(health_json["health"]["authenticated"], false);
    assert!(health_json["daemon_health"].is_null());

    let details = health_json["health"]["details"].as_str().unwrap();
    assert!(details.contains("not connected") || details.contains("not started"));
}

fn seed_gateway_clear_fixture(config: &AppConfig, artifact_path: String) {
    let db = Database::open(&config.paths.db_path).unwrap();
    let now = Utc::now();
    db.upsert_session(
        "session-clear",
        "Session",
        Some("https://chatgpt.com/"),
        now,
    )
    .unwrap();
    db.upsert_conversation(
        "conv-clear",
        "session-clear",
        "Conversation",
        Some("https://chatgpt.com/c/conv-clear"),
        Some("prompt"),
        Some("response"),
        None,
        now,
    )
    .unwrap();
    db.insert_job(
        "job-clear",
        shared_protocol::JobStatus::Completed,
        Some("complete"),
        Some("conv-clear"),
        now,
        now,
    )
    .unwrap();
    db.upsert_job_detail(
        "job-clear",
        shared_protocol::JobKind::Ask,
        Some("prompt"),
        None,
    )
    .unwrap();
    db.insert_artifact(
        &shared_protocol::ArtifactMetadata {
            artifact_id: "artifact-clear".to_owned(),
            conversation_id: Some("conv-clear".to_owned()),
            message_id: Some("msg-clear".to_owned()),
            file_name: "artifact.txt".to_owned(),
            content_type: "text/plain".to_owned(),
            size_bytes: 8,
            sha256: "sha-clear".to_owned(),
            storage_path: artifact_path,
        },
        now,
    )
    .unwrap();
    db.link_job_artifact("job-clear", "artifact-clear").unwrap();
}
