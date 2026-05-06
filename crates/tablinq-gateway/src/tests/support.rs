use super::*;
use gptd::Database;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Stdio};
use std::sync::{Mutex, OnceLock};
use tempfile::{TempDir, tempdir};

pub(super) fn integration_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(super) struct TestChild {
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

pub(super) fn spawn_fake_chatgpt() -> (TestChild, String) {
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

pub(super) fn spawn_gateway(
    config: AppConfig,
) -> (thread::JoinHandle<Result<()>>, GatewayStatusSnapshot) {
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

pub(super) struct RunningGateway {
    state_root: std::path::PathBuf,
    handle: Option<thread::JoinHandle<Result<()>>>,
    pub(super) snapshot: GatewayStatusSnapshot,
}

impl RunningGateway {
    pub(super) fn start(config: AppConfig) -> Self {
        let state_root = config.paths.state_root.clone();
        let (handle, snapshot) = spawn_gateway(config);
        Self {
            state_root,
            handle: Some(handle),
            snapshot,
        }
    }
}

impl Drop for RunningGateway {
    fn drop(&mut self) {
        let _ = stop_gateway_process(Some(&self.state_root));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub(super) fn test_config() -> (TempDir, AppConfig) {
    let dir = tempdir().unwrap();
    let mut config = AppConfig::load_or_create(Some(dir.path())).unwrap();
    config.file.browser_connection_mode = Some(shared_protocol::BrowserConnectionMode::ManagedOnly);
    config.file.browser_submit_jitter_min_ms = Some(0);
    config.file.browser_submit_jitter_max_ms = Some(0);
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.port = Some(0);
        gateway.ask_timeout_ms = Some(15_000);
        gateway.extended_thinking_timeout_ms = Some(15_000);
        gateway.research_timeout_ms = Some(15_000);
        gateway.queue_timeout_ms = Some(15_000);
    }
    (dir, config)
}

pub(super) fn auth_header(config: &AppConfig) -> String {
    format!(
        "Bearer {}",
        config.gateway_api_key().expect("gateway api key")
    )
}

pub(super) fn upload_file(
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

pub(super) fn upload_with_declared_content_length(
    snapshot: &GatewayStatusSnapshot,
    auth: &str,
    file_name: &str,
    content_type: &str,
    declared_content_length: usize,
) -> SimpleHttpResponse {
    crate::http::http_request_with_declared_content_length(
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

pub(super) fn raw_http_request(
    snapshot: &GatewayStatusSnapshot,
    raw_request: &[u8],
) -> SimpleHttpResponse {
    let mut stream = TcpStream::connect((snapshot.host.as_str(), snapshot.port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_millis(5_000)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_millis(5_000)))
        .unwrap();
    stream.write_all(raw_request).unwrap();
    stream.flush().unwrap();

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line).unwrap();
    let parts: Vec<_> = status_line.split_whitespace().collect();
    assert!(parts.len() >= 2, "invalid status line: {status_line:?}");
    let status_code = parts[1].parse::<u16>().unwrap();
    let reason_phrase = parts[2..].join(" ");

    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let (name, value) = trimmed.split_once(':').unwrap();
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }

    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).unwrap();
    } else {
        reader.read_to_end(&mut body).unwrap();
    }

    SimpleHttpResponse {
        status_code,
        reason_phrase,
        headers,
        body,
    }
}

pub(super) fn decode_json_response(response: &SimpleHttpResponse) -> Value {
    serde_json::from_slice(&response.body).unwrap()
}

pub(super) fn assert_json_status(response: &SimpleHttpResponse, status: u16) -> Value {
    assert_eq!(response.status_code, status);
    decode_json_response(response)
}

pub(super) fn request_summary_json(
    snapshot: &GatewayStatusSnapshot,
    auth: &str,
    request_id: &str,
) -> Value {
    let response = http_request(
        &snapshot.host,
        snapshot.port,
        "GET",
        &format!("/v1/requests/{request_id}"),
        &[("authorization", auth)],
        &[],
        20_000,
    )
    .unwrap();
    assert_json_status(&response, 200)
}

pub(super) fn latest_failure_bundle_request(
    state_root: &Path,
) -> shared_protocol::GatewayRequestContext {
    let config = AppConfig::load_or_create(Some(state_root)).unwrap();
    let db = Database::open(&config.paths.db_path).unwrap();
    db.latest_failure_bundle()
        .unwrap()
        .expect("failure bundle")
        .gateway_request
        .expect("gateway request context")
}

pub(super) fn openclaude_request_body(
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

pub(super) fn paperclip_request_body(
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
