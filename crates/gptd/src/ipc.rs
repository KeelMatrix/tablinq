use anyhow::{Context, Result};
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, Stream, ToNsName, traits::Stream as LocalSocketStreamTrait,
};
use shared_protocol::{DaemonFrame, DaemonRequest, DaemonResponse, EventPayload};
use std::io::{BufRead, BufReader, Write};

pub fn bind(endpoint: &str) -> Result<interprocess::local_socket::Listener> {
    let name = endpoint
        .to_ns_name::<GenericNamespaced>()
        .context("convert endpoint to local socket name")?;
    ListenerOptions::new()
        .name(name)
        .create_sync()
        .context("bind local socket listener")
}

pub fn send_request(endpoint: &str, request: &DaemonRequest) -> Result<DaemonResponse> {
    let name = endpoint
        .to_ns_name::<GenericNamespaced>()
        .context("convert endpoint to local socket name")?;
    let stream = Stream::connect(name).context("connect to local socket")?;
    round_trip(stream, request)
}

pub fn daemon_is_responsive(endpoint: &str) -> bool {
    send_request(endpoint, &DaemonRequest::Ping).is_ok()
}

pub fn send_request_stream<F>(
    endpoint: &str,
    request: &DaemonRequest,
    mut on_event: F,
) -> Result<DaemonResponse>
where
    F: FnMut(EventPayload) -> Result<()>,
{
    let name = endpoint
        .to_ns_name::<GenericNamespaced>()
        .context("convert endpoint to local socket name")?;
    let stream = Stream::connect(name).context("connect to local socket")?;

    let mut stream = stream;
    let raw = serde_json::to_string(request).context("encode daemon request")?;
    stream
        .write_all(raw.as_bytes())
        .context("write daemon request")?;
    stream.write_all(b"\n").context("write newline")?;
    stream.flush().context("flush daemon request")?;

    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).context("read daemon frame")?;
        if line.trim().is_empty() {
            anyhow::bail!("daemon closed without a response");
        }

        match serde_json::from_str::<DaemonFrame>(line.trim_end()) {
            Ok(DaemonFrame::Event(event)) => on_event(event)?,
            Ok(DaemonFrame::Response(response)) => return Ok(response),
            Err(_) => {
                let response = serde_json::from_str::<DaemonResponse>(line.trim_end())
                    .context("decode daemon response")?;
                return Ok(response);
            }
        }
    }
}

pub fn read_request(stream: Stream) -> Result<(DaemonRequest, Stream)> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).context("read daemon request")?;
    let request =
        serde_json::from_str::<DaemonRequest>(line.trim_end()).context("decode daemon request")?;
    Ok((request, reader.into_inner()))
}

pub fn write_response(mut stream: Stream, response: &DaemonResponse) -> Result<()> {
    let raw = serde_json::to_string(response).context("encode daemon response")?;
    stream
        .write_all(raw.as_bytes())
        .context("write daemon response")?;
    stream.write_all(b"\n").context("write newline")?;
    stream.flush().context("flush daemon response")?;
    Ok(())
}

pub fn write_frame(mut stream: &Stream, frame: &DaemonFrame) -> Result<()> {
    let raw = serde_json::to_string(frame).context("encode daemon frame")?;
    stream
        .write_all(raw.as_bytes())
        .context("write daemon frame")?;
    stream.write_all(b"\n").context("write newline")?;
    stream.flush().context("flush daemon frame")?;
    Ok(())
}

fn round_trip(stream: Stream, request: &DaemonRequest) -> Result<DaemonResponse> {
    let mut stream = stream;
    let raw = serde_json::to_string(request).context("encode daemon request")?;
    stream
        .write_all(raw.as_bytes())
        .context("write daemon request")?;
    stream.write_all(b"\n").context("write newline")?;
    stream.flush().context("flush daemon request")?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("read daemon response")?;
    serde_json::from_str::<DaemonResponse>(line.trim_end()).context("decode daemon response")
}
