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

fn http_request_with_declared_content_length(
    snapshot: &GatewayStatusSnapshot,
    auth: &str,
    file_name: &str,
    content_type: &str,
    declared_content_length: usize,
) -> SimpleHttpResponse {
    super::http_request_with_declared_content_length(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_FILES_PATH,
        &[
            ("authorization", auth),
            ("x-file-name", file_name),
            ("content-type", content_type),
        ],
        declared_content_length,
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


mod capabilities_provider;

mod chat_requests;

mod failure_recovery_status;

mod gateway_process;

mod scheduler_process_control;

mod uploads_artifacts;

