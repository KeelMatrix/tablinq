use anyhow::{Context, Result};
use serde_json::Value;
use std::io::Write;
use std::net::TcpStream;

pub(crate) fn sse_response_headers(request_id: &str) -> Vec<(String, String)> {
    vec![
        ("Content-Type".to_owned(), "text/event-stream".to_owned()),
        ("Cache-Control".to_owned(), "no-cache".to_owned()),
        ("Connection".to_owned(), "close".to_owned()),
        ("X-Request-Id".to_owned(), request_id.to_owned()),
    ]
}

pub(crate) fn write_sse_headers(stream: &mut TcpStream, request_id: &str) -> Result<()> {
    let headers = sse_response_headers(request_id);
    write!(stream, "HTTP/1.1 200 OK\r\n").context("write SSE headers")?;
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").context("write SSE header")?;
    }
    write!(stream, "\r\n").context("write SSE header terminator")?;
    stream.flush().context("flush SSE headers")
}

pub(crate) fn write_sse_event(stream: &mut TcpStream, value: &Value) -> Result<()> {
    write!(
        stream,
        "data: {}\n\n",
        serde_json::to_string(value).context("encode SSE event")?
    )
    .context("write SSE event")?;
    stream.flush().context("flush SSE event")
}

pub(crate) fn write_sse_done(stream: &mut TcpStream) -> Result<()> {
    write!(stream, "data: [DONE]\n\n").context("write SSE done event")?;
    stream.flush().context("flush SSE done event")
}
