use crate::browser::urls::same_target_url;
use anyhow::{Context, Result};
use serde_json::Value;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ExistingDebugTarget {
    pub(super) status: &'static str,
    pub(super) action: &'static str,
    pub(super) target_url_opened_or_focused: bool,
    pub(super) target_tab_found_before_open: bool,
    pub(super) target_tab_found: bool,
    pub(super) target_tab_focused: bool,
    pub(super) target_tab_id: Option<String>,
}

pub(super) fn focus_existing_debug_target(
    config: &gptd::AppConfig,
    target_url: &str,
) -> Option<ExistingDebugTarget> {
    let endpoint = parse_http_endpoint(&config.browser_debug_endpoint())?;
    let list = devtools_request(&endpoint, "GET", "/json/list").ok()?;
    let targets: Value = serde_json::from_str(&list).ok()?;
    let targets = targets.as_array()?;
    let target_id = targets.iter().find_map(|target| {
        let url = target.get("url").and_then(Value::as_str)?;
        if !same_target_url(url, target_url) {
            return None;
        }
        target
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
    });

    if let Some(target_id) = target_id {
        let focused =
            devtools_request(&endpoint, "GET", &format!("/json/activate/{target_id}")).is_ok();
        return Some(ExistingDebugTarget {
            status: "already_running",
            action: "reused_existing_debug_browser",
            target_url_opened_or_focused: focused,
            target_tab_found_before_open: true,
            target_tab_found: true,
            target_tab_focused: focused,
            target_tab_id: Some(target_id),
        });
    }

    let created_target_id =
        create_debug_target(&endpoint, target_url).and_then(|value| target_id_from_value(&value));
    Some(ExistingDebugTarget {
        status: "already_running",
        action: "opened_target_in_existing_debug_browser",
        target_url_opened_or_focused: created_target_id.is_some(),
        target_tab_found_before_open: false,
        target_tab_found: created_target_id.is_some(),
        target_tab_focused: false,
        target_tab_id: created_target_id,
    })
}

pub(super) fn wait_for_existing_debug_target(
    config: &gptd::AppConfig,
    target_url: &str,
) -> Option<ExistingDebugTarget> {
    for _ in 0..30 {
        if let Some(existing) = focus_existing_debug_target(config, target_url) {
            return Some(existing);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

pub(super) fn focus_existing_debug_target_before_launch(
    config: &gptd::AppConfig,
    target_url: &str,
) -> Option<ExistingDebugTarget> {
    for attempt in 0..5 {
        if let Some(existing) = focus_existing_debug_target(config, target_url) {
            return Some(existing);
        }
        if attempt < 4 {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    None
}

fn create_debug_target(endpoint: &HttpEndpoint, target_url: &str) -> Option<Value> {
    for method in ["PUT", "GET"] {
        let response = devtools_request(endpoint, method, &format!("/json/new?{target_url}")).ok();
        if let Some(response) = response
            && let Ok(value) = serde_json::from_str(&response)
        {
            return Some(value);
        }
    }
    None
}

fn target_id_from_value(value: &Value) -> Option<String> {
    value
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HttpEndpoint {
    pub(super) host: String,
    pub(super) port: u16,
}

pub(super) fn parse_http_endpoint(raw: &str) -> Option<HttpEndpoint> {
    let without_scheme = raw.strip_prefix("http://")?;
    let authority = without_scheme.split('/').next()?.trim();
    let (host, port) = authority.rsplit_once(':')?;
    Some(HttpEndpoint {
        host: host.trim_matches(['[', ']']).to_owned(),
        port: port.parse().ok()?,
    })
}

fn devtools_request(endpoint: &HttpEndpoint, method: &str, path: &str) -> Result<String> {
    let mut stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port))
        .with_context(|| format!("connect to Chrome debug endpoint {}", endpoint.port))?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(600)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(600)));
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
        endpoint.host, endpoint.port
    )
    .context("write Chrome debug endpoint request")?;
    let response =
        read_http_response(&mut stream).context("read Chrome debug endpoint response")?;
    let (head, body) = response.split_once("\r\n\r\n").unwrap_or(("", &response));
    if !head.contains(" 200 ") {
        anyhow::bail!("Chrome debug endpoint returned non-200 response");
    }
    Ok(body.to_owned())
}

fn read_http_response(stream: &mut TcpStream) -> Result<String> {
    let mut response = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                response.extend_from_slice(&chunk[..count]);
                if response_body_complete(&response) {
                    break;
                }
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
                    && !response.is_empty() =>
            {
                break;
            }
            Err(error) => return Err(error).context("read Chrome debug endpoint response"),
        }
    }
    String::from_utf8(response).context("decode Chrome debug endpoint response")
}

fn response_body_complete(response: &[u8]) -> bool {
    let Some(header_end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&response[..header_end]);
    let Some(content_length) = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    }) else {
        return false;
    };
    response.len() >= header_end + 4 + content_length
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use tempfile::tempdir;

    #[test]
    fn parses_http_debug_endpoint_authority() {
        assert_eq!(
            parse_http_endpoint("http://127.0.0.1:9222").unwrap(),
            HttpEndpoint {
                host: "127.0.0.1".to_owned(),
                port: 9222
            }
        );
        assert_eq!(
            parse_http_endpoint("http://[::1]:9223/json/version").unwrap(),
            HttpEndpoint {
                host: "::1".to_owned(),
                port: 9223
            }
        );
        assert_eq!(parse_http_endpoint("https://127.0.0.1:9222"), None);
        assert_eq!(parse_http_endpoint("http://127.0.0.1"), None);
    }

    #[test]
    fn detects_http_response_body_completion_from_content_length() {
        let complete = b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\npayload";
        let incomplete =
            b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\npayload";
        let no_length = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\npayload";

        assert!(response_body_complete(complete));
        assert!(!response_body_complete(incomplete));
        assert!(!response_body_complete(no_length));
    }

    #[test]
    fn devtools_request_returns_only_completed_response_body() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = HttpEndpoint {
            host: "127.0.0.1".to_owned(),
            port: listener.local_addr().unwrap().port(),
        };
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0_u8; 512];
            let _ = stream.read(&mut buffer).unwrap();
            let response = "HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\npayload";
            let _ = stream.write_all(response.as_bytes());
        });

        assert_eq!(
            devtools_request(&endpoint, "GET", "/json/list").unwrap(),
            "payload"
        );
    }

    #[test]
    fn focus_existing_debug_target_reuses_or_opens_target_without_live_browser() {
        let dir = tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        let (endpoint, requests) =
            start_fake_devtools_server(Some("https://chatgpt.com/c/existing"));
        std::fs::write(
            dir.path().join("config").join("config.toml"),
            format!("version = 2\nbrowser_debug_endpoint = \"{endpoint}\"\n"),
        )
        .unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();

        let existing = focus_existing_debug_target(&config, "https://chatgpt.com/").unwrap();

        assert_eq!(existing.action, "reused_existing_debug_browser");
        assert_eq!(existing.target_tab_id.as_deref(), Some("tab-1"));
        thread::sleep(Duration::from_millis(50));
        let request_lines = requests.try_iter().collect::<Vec<_>>();
        assert!(request_lines.iter().any(|line| line.contains("/json/list")));
        assert!(
            request_lines
                .iter()
                .any(|line| line.contains("/json/activate/tab-1"))
        );
    }

    fn start_fake_devtools_server(target_url: Option<&str>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let target_url = target_url.map(ToOwned::to_owned);
        thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let mut buffer = [0_u8; 2048];
                let count = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..count]).to_string();
                let request_line = request.lines().next().unwrap_or("").to_owned();
                let _ = tx.send(request_line.clone());
                let body = if request_line.contains("/json/list") {
                    target_url
                        .as_ref()
                        .map(|url| format!(r#"[{{"id":"tab-1","type":"page","url":"{url}"}}]"#))
                        .unwrap_or_else(|| "[]".to_owned())
                } else if request_line.contains("/json/new") {
                    r#"{"id":"created-tab","type":"page","url":"https://chatgpt.com/"}"#.to_owned()
                } else {
                    "{}".to_owned()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (endpoint, rx)
    }
}
