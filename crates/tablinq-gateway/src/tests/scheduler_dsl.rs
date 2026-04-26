use super::*;
use std::path::PathBuf;

pub(super) struct SchedulerGateway {
    _dir: TempDir,
    fake_server: TestChild,
    handle: Option<thread::JoinHandle<Result<()>>>,
    state_root: PathBuf,
    pub(super) auth: String,
    pub(super) snapshot: GatewayStatusSnapshot,
}

impl SchedulerGateway {
    pub(super) fn start(configure: impl FnOnce(&mut AppConfig, &str)) -> Self {
        let (fake_server, fake_url) = spawn_fake_chatgpt();
        let (dir, mut config) = test_config();
        configure(&mut config, &fake_url);
        let state_root = config.paths.state_root.clone();
        let auth = auth_header(&config);
        let (handle, snapshot) = spawn_gateway(config);
        Self {
            _dir: dir,
            fake_server,
            handle: Some(handle),
            state_root,
            auth,
            snapshot,
        }
    }

    pub(super) fn post_chat_body(
        &self,
        body: &[u8],
        request_id: Option<&str>,
        client_id: Option<&str>,
        timeout_ms: u64,
    ) -> SimpleHttpResponse {
        post_chat_completion(
            &self.snapshot,
            &self.auth,
            body,
            request_id,
            client_id,
            timeout_ms,
        )
    }

    pub(super) fn spawn_chat_body(
        &self,
        body: Vec<u8>,
        request_id: Option<&str>,
        client_id: Option<&str>,
        timeout_ms: u64,
    ) -> thread::JoinHandle<SimpleHttpResponse> {
        spawn_chat_completion(
            &self.snapshot,
            &self.auth,
            body,
            request_id,
            client_id,
            timeout_ms,
        )
    }

    pub(super) fn cancel(&self, request_id: &str, timeout_ms: u64) -> SimpleHttpResponse {
        cancel_request(&self.snapshot, &self.auth, request_id, timeout_ms)
    }

    pub(super) fn request_summary(&self, request_id: &str, timeout_ms: u64) -> SimpleHttpResponse {
        http_request(
            &self.snapshot.host,
            self.snapshot.port,
            "GET",
            &format!("/v1/requests/{request_id}"),
            &[("authorization", self.auth.as_str())],
            &[],
            timeout_ms,
        )
        .unwrap()
    }

    pub(super) fn scheduler_status(&self) -> Value {
        let response = http_request(
            &self.snapshot.host,
            self.snapshot.port,
            "GET",
            ADMIN_STATUS_PATH,
            &[("authorization", self.auth.as_str())],
            &[],
            5_000,
        )
        .unwrap();
        assert_eq!(response.status_code, 200);
        serde_json::from_slice(&response.body).unwrap()
    }

    pub(super) fn wait_for_scheduler(
        &self,
        predicate: impl Fn(&Value) -> bool,
        message: &str,
    ) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let body = self.scheduler_status();
            if predicate(&body["scheduler"]) {
                return body;
            }
            assert!(Instant::now() < deadline, "{message}");
            thread::sleep(Duration::from_millis(25));
        }
    }

    pub(super) fn shutdown(mut self) {
        self.shutdown_inner();
    }

    fn shutdown_inner(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = stop_gateway_process(Some(&self.state_root));
            let _ = handle.join();
        }
        let _ = self.fake_server.kill();
        let _ = self.fake_server.wait();
    }
}

impl Drop for SchedulerGateway {
    fn drop(&mut self) {
        self.shutdown_inner();
    }
}

pub(super) fn chat_completion_body(model: &str, prompt: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "model": model,
        "messages": [{ "role": "user", "content": prompt }]
    }))
    .unwrap()
}

pub(super) fn streaming_chat_completion_body(model: &str, prompt: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "model": model,
        "stream": true,
        "messages": [{ "role": "user", "content": prompt }]
    }))
    .unwrap()
}

pub(super) fn post_chat_completion(
    snapshot: &GatewayStatusSnapshot,
    auth: &str,
    body: &[u8],
    request_id: Option<&str>,
    client_id: Option<&str>,
    timeout_ms: u64,
) -> SimpleHttpResponse {
    let mut headers = vec![
        ("authorization", auth),
        ("content-type", "application/json"),
    ];
    if let Some(request_id) = request_id {
        headers.push(("x-request-id", request_id));
    }
    if let Some(client_id) = client_id {
        headers.push(("x-client-id", client_id));
    }
    http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        V1_CHAT_COMPLETIONS_PATH,
        &headers,
        body,
        timeout_ms,
    )
    .unwrap()
}

pub(super) fn spawn_chat_completion(
    snapshot: &GatewayStatusSnapshot,
    auth: &str,
    body: Vec<u8>,
    request_id: Option<&str>,
    client_id: Option<&str>,
    timeout_ms: u64,
) -> thread::JoinHandle<SimpleHttpResponse> {
    let host = snapshot.host.clone();
    let port = snapshot.port;
    let auth = auth.to_owned();
    let request_id = request_id.map(str::to_owned);
    let client_id = client_id.map(str::to_owned);
    thread::spawn(move || {
        let mut headers = vec![
            ("authorization", auth.as_str()),
            ("content-type", "application/json"),
        ];
        if let Some(request_id) = request_id.as_deref() {
            headers.push(("x-request-id", request_id));
        }
        if let Some(client_id) = client_id.as_deref() {
            headers.push(("x-client-id", client_id));
        }
        http_request(
            &host,
            port,
            "POST",
            V1_CHAT_COMPLETIONS_PATH,
            &headers,
            &body,
            timeout_ms,
        )
        .unwrap()
    })
}

pub(super) fn cancel_request(
    snapshot: &GatewayStatusSnapshot,
    auth: &str,
    request_id: &str,
    timeout_ms: u64,
) -> SimpleHttpResponse {
    http_request(
        &snapshot.host,
        snapshot.port,
        "POST",
        &format!("/v1/requests/{request_id}/cancel"),
        &[("authorization", auth)],
        &[],
        timeout_ms,
    )
    .unwrap()
}

pub(super) fn expect_retry_error(
    response: &SimpleHttpResponse,
    status_code: u16,
    code: &str,
) -> Value {
    assert_eq!(response.status_code, status_code);
    let body: Value = serde_json::from_slice(&response.body).unwrap();
    assert_eq!(body["error"]["code"], code);
    body
}
