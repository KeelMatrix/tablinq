use crate::routes::{V1_FILES_PATH, normalized_request_path};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use gptd::logging::JsonLogger;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub(crate) const MAX_HTTP_REQUEST_LINE_BYTES: usize = 8 * 1024;
pub(crate) const MAX_HTTP_HEADER_LINE_BYTES: usize = 8 * 1024;
pub(crate) const MAX_HTTP_HEADER_BYTES: usize = 64 * 1024;
pub(crate) const MAX_HTTP_HEADER_COUNT: usize = 100;
pub(crate) const MAX_NON_UPLOAD_REQUEST_BODY_BYTES: usize = 2 * 1024 * 1024;

pub(crate) struct RequestContext {
    pub(crate) request_id: String,
    pub(crate) request: HttpRequest,
}

#[derive(Debug)]
pub(crate) struct HttpRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct HttpResponse {
    pub(crate) status_code: u16,
    pub(crate) reason_phrase: &'static str,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct SimpleHttpResponse {
    pub(crate) status_code: u16,
    #[cfg(test)]
    pub(crate) reason_phrase: String,
    #[cfg(test)]
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct HttpParseError {
    pub(crate) status_code: u16,
    pub(crate) code: &'static str,
    pub(crate) partial: Box<PartialHttpRequest>,
    message: String,
}

#[derive(Debug, Default)]
pub(crate) struct PartialHttpRequest {
    pub(crate) request_id: Option<String>,
    pub(crate) method: Option<String>,
    pub(crate) path: Option<String>,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Option<Vec<u8>>,
}

impl std::fmt::Display for HttpParseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HttpParseError {}

pub(crate) fn log_request(
    logger: &JsonLogger,
    request_id: &str,
    request: &HttpRequest,
    remote_addr: &str,
) -> Result<()> {
    logger.info(
        "gateway request",
        json!({
            "request_id": request_id,
            "method": request.method,
            "path": request.path,
            "remote_addr": remote_addr,
            "body_len": request.body.len(),
        }),
    )
}

pub(crate) fn log_response(logger: &JsonLogger, request_id: &str, status_code: u16) -> Result<()> {
    logger.info(
        "gateway response",
        json!({
            "request_id": request_id,
            "status_code": status_code,
        }),
    )
}

pub(crate) fn json_response(status_code: u16, body: Value) -> HttpResponse {
    let body = serde_json::to_vec(&body).expect("serialize JSON response");
    HttpResponse {
        status_code,
        reason_phrase: reason_phrase_for_status(status_code),
        headers: vec![
            ("content-type".to_owned(), "application/json".to_owned()),
            ("connection".to_owned(), "close".to_owned()),
        ],
        body,
    }
}

pub(crate) fn reason_phrase_for_status(status_code: u16) -> &'static str {
    match status_code {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        405 => "Method Not Allowed",
        404 => "Not Found",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Internal Server Error",
    }
}

pub(crate) fn parse_http_request(
    stream: &mut TcpStream,
    counter: &AtomicU64,
) -> std::result::Result<RequestContext, HttpParseError> {
    let mut reader = BufReader::new(stream);
    let request_line =
        read_limited_http_line(&mut reader, MAX_HTTP_REQUEST_LINE_BYTES, "request line")?;
    if request_line.trim().is_empty() {
        return Err(bad_http_request("missing HTTP request line"));
    }

    let mut request_line_parts = request_line.split_whitespace();
    let method = request_line_parts
        .next()
        .ok_or_else(|| bad_http_request("missing HTTP method"))?
        .to_ascii_uppercase();
    let path = request_line_parts
        .next()
        .ok_or_else(|| {
            bad_http_request("missing HTTP path").with_partial(Some(method.clone()), None, None)
        })?
        .to_owned();

    let mut headers = BTreeMap::new();
    let mut total_header_bytes = 0usize;
    let mut header_count = 0usize;
    loop {
        let line =
            read_limited_http_line(&mut reader, MAX_HTTP_HEADER_LINE_BYTES, "HTTP header line")
                .map_err(|err| {
                    err.with_partial(
                        Some(method.clone()),
                        Some(path.clone()),
                        Some(headers.clone()),
                    )
                })?;
        total_header_bytes += line.len();
        if total_header_bytes > MAX_HTTP_HEADER_BYTES {
            return Err(
                bad_http_request("HTTP headers exceed gateway limit").with_partial(
                    Some(method),
                    Some(path),
                    Some(headers),
                ),
            );
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        header_count += 1;
        if header_count > MAX_HTTP_HEADER_COUNT {
            return Err(bad_http_request("too many HTTP headers").with_partial(
                Some(method),
                Some(path),
                Some(headers),
            ));
        }
        let (name, value) = trimmed.split_once(':').ok_or_else(|| {
            bad_http_request("invalid HTTP header line").with_partial(
                Some(method.clone()),
                Some(path.clone()),
                Some(headers.clone()),
            )
        })?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }

    if headers.contains_key("transfer-encoding") {
        return Err(
            bad_http_request("unsupported Transfer-Encoding").with_partial(
                Some(method),
                Some(path),
                Some(headers),
            ),
        );
    }

    let content_length = headers
        .get("content-length")
        .map(|value| {
            value.parse::<usize>().map_err(|err| {
                bad_http_request(format!("parse Content-Length: {err}")).with_partial(
                    Some(method.clone()),
                    Some(path.clone()),
                    Some(headers.clone()),
                )
            })
        })
        .transpose()?
        .unwrap_or(0);
    let request_path = normalized_request_path(&path);
    if method == "POST" && request_path == V1_FILES_PATH {
        if content_length > crate::uploads::max_staged_upload_bytes() {
            return Err(payload_too_large(
                "payload_too_large",
                crate::uploads::staged_upload_limit_message("file exceeds"),
            )
            .with_partial(Some(method), Some(path), Some(headers)));
        }
    } else if content_length > MAX_NON_UPLOAD_REQUEST_BODY_BYTES {
        return Err(
            payload_too_large("payload_too_large", "request body exceeds gateway limit")
                .with_partial(Some(method), Some(path), Some(headers)),
        );
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        reader.read_exact(&mut body).map_err(|err| {
            bad_http_request(format!("read HTTP request body: {err}")).with_partial_and_body(
                Some(method.clone()),
                Some(path.clone()),
                Some(headers.clone()),
                Some(body.clone()),
            )
        })?;
    }

    let request_id = headers
        .get("x-request-id")
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| next_request_id(counter));

    Ok(RequestContext {
        request_id,
        request: HttpRequest {
            method,
            path,
            headers,
            body,
        },
    })
}

pub(crate) fn read_limited_http_line<R: Read>(
    reader: &mut R,
    limit: usize,
    label: &'static str,
) -> std::result::Result<String, HttpParseError> {
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let count = reader
            .read(&mut byte)
            .map_err(|err| bad_http_request(format!("read {label}: {err}")))?;
        if count == 0 {
            if bytes.is_empty() {
                return Err(bad_http_request(format!("missing {label}")));
            }
            break;
        }
        bytes.push(byte[0]);
        if bytes.len() > limit {
            return Err(bad_http_request(format!("{label} exceeds gateway limit")));
        }
        if byte[0] == b'\n' {
            break;
        }
    }
    String::from_utf8(bytes).map_err(|err| bad_http_request(format!("decode {label}: {err}")))
}

pub(crate) fn bad_http_request(message: impl Into<String>) -> HttpParseError {
    HttpParseError {
        status_code: 400,
        code: "bad_request",
        partial: Box::default(),
        message: message.into(),
    }
}

pub(crate) fn payload_too_large(code: &'static str, message: impl Into<String>) -> HttpParseError {
    HttpParseError {
        status_code: 413,
        code,
        partial: Box::default(),
        message: message.into(),
    }
}

impl HttpParseError {
    fn with_partial(
        self,
        method: Option<String>,
        path: Option<String>,
        headers: Option<BTreeMap<String, String>>,
    ) -> Self {
        self.with_partial_and_body(method, path, headers, None)
    }

    fn with_partial_and_body(
        mut self,
        method: Option<String>,
        path: Option<String>,
        headers: Option<BTreeMap<String, String>>,
        body: Option<Vec<u8>>,
    ) -> Self {
        let headers = headers.unwrap_or_default();
        self.partial.request_id = headers
            .get("x-request-id")
            .cloned()
            .filter(|value| !value.trim().is_empty());
        self.partial.method = method;
        self.partial.path = path;
        self.partial.headers = headers;
        self.partial.body = body;
        self
    }
}

pub(crate) fn finalized_http_response_headers(
    request_id: &str,
    response: &HttpResponse,
) -> Vec<(String, String)> {
    let mut headers = response.headers.clone();
    if !headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("connection"))
    {
        headers.push(("Connection".to_owned(), "close".to_owned()));
    }
    headers.push(("x-request-id".to_owned(), request_id.to_owned()));
    headers.push(("content-length".to_owned(), response.body.len().to_string()));
    headers
}

pub(crate) fn write_http_response(
    stream: &mut TcpStream,
    request_id: &str,
    response: HttpResponse,
) -> Result<()> {
    let headers = finalized_http_response_headers(request_id, &response);

    let mut bytes = Vec::with_capacity(128 + headers.len() * 48 + response.body.len());
    write!(
        &mut bytes,
        "HTTP/1.1 {} {}\r\n",
        response.status_code, response.reason_phrase
    )
    .context("encode HTTP status line")?;
    for (name, value) in &headers {
        write!(&mut bytes, "{name}: {value}\r\n").context("encode HTTP header")?;
    }
    write!(&mut bytes, "\r\n").context("encode HTTP header terminator")?;
    bytes.extend_from_slice(&response.body);

    stream
        .write_all(&bytes)
        .context("write complete HTTP response")?;
    stream.flush().context("flush HTTP response")?;
    let _ = stream.shutdown(Shutdown::Write);
    Ok(())
}

pub(crate) fn next_request_id(counter: &AtomicU64) -> String {
    let seq = counter.fetch_add(1, Ordering::SeqCst) + 1;
    format!("req-{}-{seq}", Utc::now().timestamp_millis())
}

pub(crate) fn http_request(
    host: &str,
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
    timeout_ms: u64,
) -> Result<SimpleHttpResponse> {
    let mut stream = TcpStream::connect((host, port))
        .with_context(|| format!("connect to gateway {host}:{port}"))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(timeout_ms)))
        .context("set gateway client read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_millis(timeout_ms)))
        .context("set gateway client write timeout")?;

    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    )
    .context("write gateway request line")?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").context("write gateway request header")?;
    }
    write!(stream, "\r\n").context("write gateway request terminator")?;
    if !body.is_empty() {
        stream
            .write_all(body)
            .context("write gateway request body")?;
    }
    stream.flush().context("flush gateway request")?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .context("read gateway status line")?;
    let parts: Vec<_> = status_line.split_whitespace().collect();
    if parts.len() < 2 {
        bail!("invalid gateway HTTP status line");
    }
    let status_code = parts[1]
        .parse::<u16>()
        .context("parse gateway status code")?;
    #[cfg(test)]
    let reason_phrase = parts[2..].join(" ");

    let mut response_headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .context("read gateway response header")?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let (name, value) = trimmed
            .split_once(':')
            .context("invalid gateway response header")?;
        response_headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }

    let content_length = response_headers
        .get("content-length")
        .map(|value| {
            value
                .parse::<usize>()
                .context("parse response Content-Length")
        })
        .transpose()?
        .unwrap_or(0);
    let mut response_body = vec![0u8; content_length];
    if content_length > 0 {
        reader
            .read_exact(&mut response_body)
            .context("read gateway response body")?;
    } else {
        reader
            .read_to_end(&mut response_body)
            .context("read gateway response body to end")?;
    }

    Ok(SimpleHttpResponse {
        status_code,
        #[cfg(test)]
        reason_phrase,
        #[cfg(test)]
        headers: response_headers,
        body: response_body,
    })
}

#[cfg(test)]
pub(crate) fn http_request_with_declared_content_length(
    host: &str,
    port: u16,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    declared_content_length: usize,
    timeout_ms: u64,
) -> Result<SimpleHttpResponse> {
    let mut stream = TcpStream::connect((host, port))
        .with_context(|| format!("connect to gateway {host}:{port}"))?;
    stream
        .set_read_timeout(Some(Duration::from_millis(timeout_ms)))
        .context("set gateway client read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_millis(timeout_ms)))
        .context("set gateway client write timeout")?;

    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Length: {declared_content_length}\r\nConnection: close\r\n"
    )
    .context("write gateway request line")?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").context("write gateway request header")?;
    }
    write!(stream, "\r\n").context("write gateway request terminator")?;
    stream.flush().context("flush gateway request")?;

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .context("read gateway status line")?;
    let parts: Vec<_> = status_line.split_whitespace().collect();
    if parts.len() < 2 {
        bail!("invalid gateway HTTP status line");
    }
    let status_code = parts[1]
        .parse::<u16>()
        .context("parse gateway status code")?;
    #[cfg(test)]
    let reason_phrase = parts[2..].join(" ");

    let mut response_headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .context("read gateway response header")?;
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let (name, value) = trimmed
            .split_once(':')
            .context("invalid gateway response header")?;
        response_headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }

    let content_length = response_headers
        .get("content-length")
        .map(|value| {
            value
                .parse::<usize>()
                .context("parse response Content-Length")
        })
        .transpose()?
        .unwrap_or(0);
    let mut response_body = vec![0u8; content_length];
    if content_length > 0 {
        reader
            .read_exact(&mut response_body)
            .context("read gateway response body")?;
    } else {
        reader
            .read_to_end(&mut response_body)
            .context("read gateway response body to end")?;
    }

    Ok(SimpleHttpResponse {
        status_code,
        #[cfg(test)]
        reason_phrase,
        #[cfg(test)]
        headers: response_headers,
        body: response_body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Read};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn forbidden_status_uses_standard_reason_phrase() {
        assert_eq!(reason_phrase_for_status(403), "Forbidden");
    }

    #[test]
    fn write_http_response_adds_connection_close_to_manual_responses() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let writer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            write_http_response(
                &mut stream,
                "req-write",
                HttpResponse {
                    status_code: 200,
                    reason_phrase: "OK",
                    headers: vec![("Content-Type".to_owned(), "application/json".to_owned())],
                    body: b"{}".to_vec(),
                },
            )
            .unwrap();
        });

        let stream = TcpStream::connect(addr).unwrap();
        let mut text = String::new();
        BufReader::new(stream).read_to_string(&mut text).unwrap();
        writer.join().unwrap();

        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(text.contains("\r\nConnection: close\r\n"));
        assert!(text.ends_with("\r\n\r\n{}"));
    }
}
