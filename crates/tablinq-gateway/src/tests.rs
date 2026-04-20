use super::*;
use gptd::Database;
use gptd::browser::browser_integration_prereqs_available;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use tempfile::{TempDir, tempdir};

fn integration_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

macro_rules! require_browser_integration_prereqs {
    () => {
        if !browser_integration_prereqs_available() {
            eprintln!(
                "skipping browser-backed gateway test: browser adapter prerequisites are unavailable"
            );
            return;
        }
    };
}

struct TestChild {
    child: Child,
}

impl TestChild {
    fn spawn(mut command: Command) -> Self {
        let child = command.spawn().expect("spawn test child");
        Self { child }
    }
}

impl std::ops::Deref for TestChild {
    type Target = Child;

    fn deref(&self) -> &Self::Target {
        &self.child
    }
}

impl std::ops::DerefMut for TestChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

impl Drop for TestChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_fake_chatgpt() -> (TestChild, String) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("allocate tcp port");
    let port = listener.local_addr().expect("resolve local address").port();
    drop(listener);

    let server_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("packages")
        .join("fake-chatgpt")
        .join("server.mjs");
    let mut command = Command::new("node");
    command
        .arg(server_path)
        .env("PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = TestChild::spawn(command);

    let url = format!("http://127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return (child, url);
        }
        thread::sleep(Duration::from_millis(100));
    }

    panic!("fake-chatgpt did not become ready on port {port}");
}

fn spawn_gateway(config: AppConfig) -> (thread::JoinHandle<Result<()>>, GatewayStatusSnapshot) {
    let state_root = config.paths.state_root.clone();
    let handle = thread::spawn(move || run_gateway(config));
    let config = AppConfig::load_or_create(Some(&state_root)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(snapshot) = load_status_snapshot(&config).unwrap() {
            return (handle, snapshot);
        }
        assert!(
            Instant::now() < deadline,
            "gateway did not write status snapshot"
        );
        thread::sleep(Duration::from_millis(50));
    }
}

fn spawn_daemon(config: AppConfig) -> thread::JoinHandle<Result<()>> {
    let endpoint = shared_protocol::build_state_paths(&config.paths.state_root).ipc_endpoint;
    let handle = thread::spawn(move || gptd::run_service(config));
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if gptd::daemon_is_responsive(&endpoint) {
            return handle;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("daemon did not become ready");
}

fn stop_daemon(state_root: &Path, handle: thread::JoinHandle<Result<()>>) {
    let endpoint = shared_protocol::build_state_paths(state_root).ipc_endpoint;
    let response =
        gptd::send_request(&endpoint, &shared_protocol::DaemonRequest::Shutdown).unwrap();
    match response {
        shared_protocol::DaemonResponse::Ack(_) => {}
        other => panic!("unexpected daemon shutdown response: {other:?}"),
    }
    handle.join().unwrap().unwrap();
}

fn test_config() -> (TempDir, AppConfig) {
    let dir = tempdir().unwrap();
    let mut config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    config.file.browser_connection_mode =
        Some(shared_protocol::BrowserConnectionMode::ManagedOnly);
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.port = Some(0);
        gateway.ask_timeout_ms = Some(15_000);
        gateway.extended_thinking_timeout_ms = Some(15_000);
        gateway.research_timeout_ms = Some(15_000);
        gateway.queue_timeout_ms = Some(15_000);
    }
    (dir, config)
}

fn auth_header(config: &AppConfig) -> String {
    format!(
        "Bearer {}",
        config.gateway_api_key().expect("gateway api key")
    )
}

fn upload_file(
    snapshot: &GatewayStatusSnapshot,
    auth: &str,
    file_name: &str,
    content_type: &str,
    body: &[u8],
) -> SimpleHttpResponse {
    http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_FILES_PATH,
        &[
            ("authorization", auth),
            ("x-file-name", file_name),
            ("content-type", content_type),
        ],
        body,
        20_000,
    )
    .unwrap()
}

fn openclaude_request_body(
    model: &str,
    prompt: &str,
    role: &str,
    stream: bool,
    parent_request_id: Option<&str>,
) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "model": model,
        "stream": stream,
        "tablinq_request_role": role,
        "tablinq_parent_request_id": parent_request_id,
        "messages": [
            { "role": "user", "content": prompt }
        ]
    }))
    .unwrap()
}

fn paperclip_request_body(
    model: &str,
    prompt: &str,
    stream: bool,
    file_ids: &[&str],
) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "model": model,
        "stream": stream,
        "tablinq_file_ids": file_ids,
        "messages": [
            { "role": "user", "content": prompt }
        ]
    }))
    .unwrap()
}

#[test]
fn upload_store_cleans_up_expired_staged_files() {
    let dir = tempdir().unwrap();
    let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    let store = UploadStore::load(&config).unwrap();
    let metadata = store
        .stage_upload("..\\unsafe?.txt", Some("text/plain"), b"hello")
        .unwrap();
    assert_eq!(metadata.file_name, "unsafe_.txt");

    let upload_dir = gateway_uploads_dir(&config).join(&metadata.file_id);
    let expired = StagedFileMetadata {
        expires_at: Utc::now() - chrono::Duration::seconds(1),
        ..metadata
    };
    fs::write(
        staged_metadata_path(&upload_dir),
        serde_json::to_vec_pretty(&expired).unwrap(),
    )
    .unwrap();

    store.cleanup_expired().unwrap();
    assert!(!upload_dir.exists());
}

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
    assert_eq!(body["object"], "tablinq.provider");
    assert_eq!(body["capability_source"], "live");
    assert_eq!(body["request_annotations"]["field"], "tablinq_request_role");
    assert_eq!(
        body["provider_capabilities"]["capabilities"]["search"]["availability"],
        "available"
    );
    assert_eq!(
        body["provider_capabilities"]["capabilities"]["research"]["availability"],
        "available"
    );
    assert_eq!(
        body["provider_capabilities"]["capabilities"]["uploads"]["availability"],
        "available"
    );
    assert_eq!(
        body["provider_capabilities"]["capabilities"]["artifacts"]["availability"],
        "available"
    );
    assert_eq!(
        body["provider_capabilities"]["max_safe_parallelism"]["requests"],
        1
    );
    assert_eq!(
        body["provider_capabilities"]["subagent_recommendation"]["policy"],
        "disabled"
    );
    assert_eq!(body["presets"]["openclaude"]["max_parallel_requests"], 1);
    assert_eq!(
        body["presets"]["openclaude"]["subagents"],
        "disabled_by_default"
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
    assert_eq!(
        body["presets"]["paperclip"]["transport"],
        "openai_compatible_http"
    );
    assert_eq!(body["presets"]["paperclip"]["max_parallel_requests"], 1);
    assert_eq!(body["presets"]["paperclip"]["files"], true);
    assert_eq!(body["presets"]["paperclip"]["artifacts"], true);
    assert_eq!(
        body["presets"]["paperclip"]["subagent_recommendation"],
        "disable"
    );
    assert_eq!(
        body["presets"]["paperclip"]["usage_accounting"],
        "unknown_or_estimated"
    );

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
    assert_eq!(
        provider_body["request_annotations"]["field"],
        "tablinq_request_role"
    );
    assert_eq!(
        provider_body["request_annotations"]["parent_request_id_field"],
        "tablinq_parent_request_id"
    );
    assert_eq!(
        provider_body["presets"]["openclaude"]["max_parallel_requests"],
        1
    );
    assert_eq!(
        provider_body["presets"]["paperclip"]["max_parallel_requests"],
        1
    );
    assert_eq!(
        provider_body["presets"]["paperclip"]["subagent_recommendation"],
        "disable"
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
fn openclaude_primary_streaming_requests_keep_sse_shape() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(fake_url);
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let body =
        openclaude_request_body("tablinq-search", "latest roadmap", "primary", true, None);
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
        200,
        "{}",
        String::from_utf8_lossy(&response.body)
    );
    let text = String::from_utf8(response.body).unwrap();
    assert!(text.contains("chat.completion.chunk"));
    assert!(text.contains("\"tablinq_request_role\":\"primary\""));
    assert!(text.contains("[DONE]"));

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn openclaude_subagent_requests_are_rejected_by_default_policy() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let body = openclaude_request_body(
        "tablinq-instant",
        "parallel helper request",
        "subagent",
        false,
        Some("oc-primary-1"),
    );
    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-request-id", "oc-subagent-1"),
        ],
        &body,
        20_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 429);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["error"]["code"], "subagents_disabled");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
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
    assert_eq!(body["object"], "list");
    assert_eq!(body["tablinq_capability_source"], "live");
    assert_eq!(
        body["tablinq_model_ids"],
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
    assert_eq!(models[1]["id"], "tablinq-thinking-standard");
    assert_eq!(models[1]["tablinq_model_class"], "thinking");
    assert_eq!(models[1]["tablinq_effort"], "standard");
    assert_eq!(models[2]["id"], "tablinq-thinking-extended");
    assert_eq!(models[2]["tablinq_effort"], "extended");
    assert_eq!(models[3]["id"], "tablinq-search");
    assert_eq!(models[3]["tablinq_mode"], "search");
    assert_eq!(models[4]["id"], "tablinq-research");
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
    assert_eq!(body["object"], "tablinq.capability_catalog");
    assert_eq!(body["source"], "live");
    assert_eq!(
        body["provider"]["available_model_ids"],
        json!([
            "tablinq-instant",
            "tablinq-thinking-standard",
            "tablinq-thinking-extended",
            "tablinq-search",
            "tablinq-research",
        ])
    );
    assert!(body["models"].as_array().unwrap().iter().any(|item| {
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
    config.file.browser_target_url =
        Some(format!("{fake_url}/?missing=effort,search,research"));
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
    let search = body["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "tablinq-search")
        .unwrap();
    let thinking = body["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "tablinq-thinking-standard")
        .unwrap();
    let research = body["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == "tablinq-research")
        .unwrap();
    assert_eq!(thinking["tablinq_availability"], "unavailable");
    assert_eq!(search["tablinq_availability"], "unavailable");
    assert_eq!(research["tablinq_availability"], "unavailable");
    assert!(!search["tablinq_warning"].as_array().unwrap().is_empty());
    assert_eq!(
        body["provider"]["capabilities"]["search"]["availability"],
        "unavailable"
    );
    assert_eq!(
        body["provider"]["capabilities"]["research"]["availability"],
        "unavailable"
    );
    let available_ids = body["provider"]["available_model_ids"].as_array().unwrap();
    assert_eq!(available_ids.len(), 1);
    assert_eq!(available_ids[0], "tablinq-instant");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

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
    let err = anyhow::anyhow!("auth required: managed browser session is not authenticated");
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
            .any(|value| value.as_str().is_some_and(|text| text.contains("sign in")))
    );
}

#[test]
fn files_upload_endpoint_stages_files_and_research_accepts_file_ids() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let upload = upload_file(
        &snapshot,
        &auth,
        "brief.txt",
        "text/plain",
        b"customer brief",
    );
    assert_eq!(upload.status_code, 201);
    let upload_body: Value = serde_json::from_slice(&upload.body).unwrap();
    let file_id = upload_body["data"]["id"].as_str().unwrap().to_owned();
    assert_eq!(upload_body["data"]["file_name"], "brief.txt");
    assert_eq!(upload_body["data"]["content_type"], "text/plain");
    assert_eq!(upload_body["data"]["size_bytes"], 14);
    assert!(upload_body["data"]["sha256"].as_str().is_some());
    assert!(
        upload_body["data"]["linked_request_ids"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let first_body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "tablinq_file_ids": [file_id],
        "messages": [
            { "role": "user", "content": "summarize the upload" }
        ]
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
    assert_eq!(first.status_code, 200);
    let first_json: Value = serde_json::from_slice(&first.body).unwrap();
    assert!(
        first_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap()
            .contains("brief.txt")
    );
    let conversation_id = first_json["tablinq_conversation_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let second_body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "tablinq_file_ids": [file_id],
        "tablinq_conversation_id": conversation_id,
        "messages": [
            { "role": "user", "content": "reuse the same upload again" }
        ]
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
            ("x-request-id", "req-file-reuse-2"),
        ],
        &second_body,
        20_000,
    )
    .unwrap();
    assert_eq!(second.status_code, 200);

    let detail = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        &format!("{V1_FILES_PATH}/{file_id}"),
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(detail.status_code, 200);
    let detail_json: Value = serde_json::from_slice(&detail.body).unwrap();
    assert_eq!(detail_json["data"]["id"], file_id);
    assert!(
        detail_json["data"]["linked_request_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "req-file-reuse-2")
    );
    assert!(
        detail_json["data"]["linked_conversation_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.as_str().is_some())
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn paperclip_like_http_flow_supports_files_and_artifacts() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let upload = upload_file(
        &snapshot,
        &auth,
        "paperclip-source.txt",
        "text/plain",
        b"paperclip source text",
    );
    assert_eq!(upload.status_code, 201);
    let upload_body: Value = serde_json::from_slice(&upload.body).unwrap();
    let file_id = upload_body["data"]["id"].as_str().unwrap().to_owned();

    let body = paperclip_request_body(
        "tablinq-research",
        "produce a summary report from the staged file",
        false,
        &[file_id.as_str()],
    );
    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-client-id", "paperclip"),
        ],
        &body,
        20_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 200);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    let job_id = body["tablinq_job_id"].as_str().unwrap().to_owned();
    assert!(
        body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value.as_str()
                == Some("usage is unavailable for the browser-backed provider"))
    );

    let artifacts = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_ARTIFACTS_PATH,
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(artifacts.status_code, 200);
    let artifacts_body: Value = serde_json::from_slice(&artifacts.body).unwrap();
    let artifact = artifacts_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["linked_job_ids"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == &Value::String(job_id.clone()))
        })
        .unwrap();
    let content = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        artifact["content_url"].as_str().unwrap(),
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(content.status_code, 200);
    assert!(
        String::from_utf8(content.body)
            .unwrap()
            .contains("paperclip-source.txt")
    );

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn provider_smoke_covers_chat_files_artifacts_and_cancel_flow() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let chat = http_request(
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
            "messages": [{ "role": "user", "content": "provider smoke ask" }]
        }))
        .unwrap(),
        20_000,
    )
    .unwrap();
    assert_eq!(chat.status_code, 200);

    let upload = upload_file(
        &snapshot,
        &auth,
        "smoke.txt",
        "text/plain",
        b"provider smoke file",
    );
    assert_eq!(upload.status_code, 201);
    let upload_body: Value = serde_json::from_slice(&upload.body).unwrap();
    let file_id = upload_body["data"]["id"].as_str().unwrap().to_owned();

    let artifact_response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ],
        &paperclip_request_body(
            "tablinq-research",
            "create a smoke artifact",
            false,
            &[&file_id],
        ),
        20_000,
    )
    .unwrap();
    assert_eq!(artifact_response.status_code, 200);
    let artifact_response_body: Value =
        serde_json::from_slice(&artifact_response.body).unwrap();
    let request_id = artifact_response_body["request_id"]
        .as_str()
        .unwrap()
        .to_owned();

    let artifacts = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_ARTIFACTS_PATH,
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(artifacts.status_code, 200);
    let artifacts_body: Value = serde_json::from_slice(&artifacts.body).unwrap();
    let artifact_id = artifacts_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["linked_request_ids"].as_array().is_some_and(|linked| {
                linked
                    .iter()
                    .any(|value| value == &Value::String(request_id.clone()))
            })
        })
        .and_then(|item| item["id"].as_str())
        .unwrap()
        .to_owned();

    let artifact_detail = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        &format!("{V1_ARTIFACTS_PATH}/{artifact_id}"),
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(artifact_detail.status_code, 200);

    let artifact_content = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        &format!("{V1_ARTIFACTS_PATH}/{artifact_id}/content"),
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(artifact_content.status_code, 200);
    assert!(
        String::from_utf8(artifact_content.body)
            .unwrap()
            .contains("smoke.txt")
    );

    let cancel_body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "messages": [{ "role": "user", "content": "cancel this smoke request" }]
    }))
    .unwrap();
    let host = snapshot.host.clone();
    let auth_for_thread = auth.clone();
    let running = thread::spawn(move || {
        http_request(
            &host,
            snapshot.port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &[
                ("authorization", auth_for_thread.as_str()),
                ("content-type", "application/json"),
                ("x-request-id", "req-provider-smoke-cancel"),
            ],
            &cancel_body,
            20_000,
        )
        .unwrap()
    });

    thread::sleep(Duration::from_millis(500));
    let cancel = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        "/v1/requests/req-provider-smoke-cancel/cancel",
        &[("authorization", auth.as_str())],
        &[],
        5_000,
    )
    .unwrap();
    assert_eq!(cancel.status_code, 200);
    let running = running.join().unwrap();
    assert_eq!(running.status_code, 409);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn paperclip_like_requests_report_missing_staged_file_errors_cleanly() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let body = paperclip_request_body(
        "tablinq-instant",
        "read the staged file",
        false,
        &["file-missing"],
    );
    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &[
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
            ("x-client-id", "paperclip"),
        ],
        &body,
        20_000,
    )
    .unwrap();
    assert_eq!(response.status_code, 400);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["error"]["code"], "file_not_found");

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
}

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
fn artifacts_endpoints_list_metadata_and_stream_content() {
    let _guard = integration_lock();
    require_browser_integration_prereqs!();
    let (mut fake_server, fake_url) = spawn_fake_chatgpt();
    let (_dir, mut config) = test_config();
    config.file.browser_target_url = Some(format!("{fake_url}/?scenario=research-success"));
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let upload = upload_file(
        &snapshot,
        &auth,
        "report.txt",
        "text/plain",
        b"artifact seed",
    );
    assert_eq!(upload.status_code, 201);
    let upload_body: Value = serde_json::from_slice(&upload.body).unwrap();
    let file_id = upload_body["data"]["id"].as_str().unwrap().to_owned();

    let body = serde_json::to_vec(&json!({
        "model": "tablinq-research",
        "tablinq_file_ids": [file_id],
        "messages": [
            { "role": "user", "content": "produce a downloadable report" }
        ]
    }))
    .unwrap();
    let completion = http_request(
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
    assert_eq!(completion.status_code, 200);
    let completion_body: Value = serde_json::from_slice(&completion.body).unwrap();
    let job_id = completion_body["tablinq_job_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let request_id = completion_body["request_id"].as_str().unwrap().to_owned();

    let artifacts = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_ARTIFACTS_PATH,
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(artifacts.status_code, 200);
    let artifacts_body: Value = serde_json::from_slice(&artifacts.body).unwrap();
    let artifact = artifacts_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["linked_job_ids"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == &Value::String(job_id.clone()))
        })
        .unwrap();
    let artifact_id = artifact["id"].as_str().unwrap().to_owned();

    let detail = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        &format!("{V1_ARTIFACTS_PATH}/{artifact_id}"),
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(detail.status_code, 200);
    let detail_body: Value = serde_json::from_slice(&detail.body).unwrap();
    assert_eq!(detail_body["data"]["id"], artifact_id);
    assert!(
        detail_body["data"]["linked_job_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == &Value::String(job_id.clone()))
    );
    assert!(
        detail_body["data"]["linked_request_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == &Value::String(request_id.clone()))
    );
    assert!(detail_body["data"]["created_at"].as_str().is_some());
    assert!(detail_body["data"]["sha256"].as_str().is_some());

    let content = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        &format!("{V1_ARTIFACTS_PATH}/{artifact_id}/content"),
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(content.status_code, 200);
    let text = String::from_utf8(content.body).unwrap();
    assert!(text.contains("report.txt"));

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
    let _ = fake_server.kill();
    let _ = fake_server.wait();
}

#[test]
fn files_endpoint_keeps_duplicate_original_names_as_separate_staged_ids() {
    let _guard = integration_lock();
    let (_dir, config) = test_config();
    let state_root = config.paths.state_root.clone();
    let auth = auth_header(&config);
    let (handle, snapshot) = spawn_gateway(config);

    let first = upload_file(&snapshot, &auth, "same.txt", "text/plain", b"one");
    let second = upload_file(&snapshot, &auth, "same.txt", "text/plain", b"two");
    assert_eq!(first.status_code, 201);
    assert_eq!(second.status_code, 201);

    let files = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        V1_FILES_PATH,
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(files.status_code, 200);
    let body: Value = serde_json::from_slice(&files.body).unwrap();
    let matching = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["file_name"] == "same.txt")
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 2);
    assert_ne!(matching[0]["id"], matching[1]["id"]);
    assert_eq!(matching[0]["file_name"], "same.txt");
    assert_eq!(matching[1]["file_name"], "same.txt");
    assert_ne!(matching[0]["sha256"], matching[1]["sha256"]);

    stop_gateway_process(Some(&state_root)).unwrap();
    handle.join().unwrap().unwrap();
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
        &anyhow::anyhow!("auth required: managed browser session is not authenticated"),
        Some("bundle-auth-1"),
    );
    assert_eq!(response.status_code, 503);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["error"]["code"], "auth_required");
    assert_eq!(body["tablinq_refusal_reason"], "auth_required");
    assert_eq!(body["tablinq_failure_bundle_id"], "bundle-auth-1");
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

    let summary = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        "/v1/requests/req-role-timeout",
        &[("authorization", auth.as_str())],
        &[],
        20_000,
    )
    .unwrap();
    assert_eq!(summary.status_code, 200);
    let summary_body: Value = serde_json::from_slice(&summary.body).unwrap();
    assert_eq!(summary_body["data"]["request_role"], "branch");
    assert_eq!(summary_body["data"]["parent_request_id"], "req-parent-1");

    let config = AppConfig::load_or_create(Some(&state_root)).unwrap();
    let db = Database::open(&config.paths.db_path).unwrap();
    let bundle = db.latest_failure_bundle().unwrap().expect("failure bundle");
    let gateway_request = bundle.gateway_request.expect("gateway request context");
    assert_eq!(gateway_request.request_id, "req-role-timeout");
    assert_eq!(gateway_request.request_role.as_deref(), Some("branch"));
    assert_eq!(
        gateway_request.parent_request_id.as_deref(),
        Some("req-parent-1")
    );

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
