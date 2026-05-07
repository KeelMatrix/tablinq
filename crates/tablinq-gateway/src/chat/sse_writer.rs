use super::PreparedChatRequest;
use crate::http::RequestContext;
use crate::openai::request_role_name;
use crate::sse::{write_sse_done, write_sse_event, write_sse_headers};
use anyhow::Result;
use chrono::Utc;
use serde_json::{Value, json};
use shared_protocol::{EventPayload, StreamChunkKind};
use std::net::TcpStream;

pub(super) fn write_stream_headers(stream: &mut TcpStream, request_id: &str) -> Result<()> {
    write_sse_headers(stream, request_id)
}

pub(super) fn write_stream_role_chunk(
    stream: &mut TcpStream,
    stream_id: &str,
    prepared: &PreparedChatRequest,
    context: &RequestContext,
) -> Result<()> {
    write_sse_event(
        stream,
        &json!({
            "id": stream_id,
            "object": "chat.completion.chunk",
            "created": Utc::now().timestamp(),
            "model": prepared.model_id,
            "request_id": context.request_id,
            "tablinq_job_id": prepared.job_id(),
            "tablinq_request_role": request_role_name(prepared.request_role),
            "tablinq_parent_request_id": prepared.parent_request_id.as_deref(),
            "choices": [{
                "index": 0,
                "delta": { "role": "assistant" },
                "finish_reason": Value::Null,
            }],
        }),
    )
}

pub(super) fn write_stream_delta(
    stream: &mut TcpStream,
    stream_id: &str,
    model_id: &str,
    request_id: &str,
    event: EventPayload,
) -> Result<()> {
    if let EventPayload::StreamChunk(chunk) = event
        && chunk.kind == StreamChunkKind::Delta
        && !chunk.text.is_empty()
    {
        write_sse_event(
            stream,
            &json!({
                "id": stream_id,
                "object": "chat.completion.chunk",
                "created": Utc::now().timestamp(),
                "model": model_id,
                "request_id": request_id,
                "tablinq_conversation_id": chunk.conversation_id,
                "choices": [{
                    "index": 0,
                    "delta": { "content": chunk.text },
                    "finish_reason": Value::Null,
                }],
            }),
        )?;
    }
    Ok(())
}

pub(super) fn write_stream_finish_chunk(
    stream: &mut TcpStream,
    stream_id: &str,
    prepared: &PreparedChatRequest,
    context: &RequestContext,
) -> Result<()> {
    write_sse_event(
        stream,
        &json!({
            "id": stream_id,
            "object": "chat.completion.chunk",
            "created": Utc::now().timestamp(),
            "model": prepared.model_id,
            "request_id": context.request_id,
            "tablinq_job_id": prepared.job_id(),
            "tablinq_request_role": request_role_name(prepared.request_role),
            "tablinq_parent_request_id": prepared.parent_request_id.as_deref(),
            "choices": [{
                "index": 0,
                "delta": {},
                "finish_reason": "stop",
            }],
        }),
    )
}

pub(super) fn write_stream_error(stream: &mut TcpStream, error: &Value) -> Result<()> {
    let mut event = error.clone();
    if event.get("object").is_none() {
        event["object"] = Value::String("error".to_owned());
    }
    write_sse_event(stream, &event)
}

pub(super) fn write_stream_done(stream: &mut TcpStream) -> Result<()> {
    write_sse_done(stream)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use shared_protocol::StreamChunk;
    use std::io::Read;
    use std::net::TcpListener;
    use std::thread;

    fn raw_assistant_text() -> &'static str {
        "raw \u{1b}[31mred\u{1b}[0m \u{1b}]0;title\u{7} \
         before\rrewritten\u{8}x\0 bidi \u{202e}spoof\u{202c}"
    }

    fn assert_no_cli_display_markers(value: &str) {
        for marker in [
            "<ESC>", "<BEL>", "<CR>", "<BS>", "<NUL>", "<U+202E>", "<U+202C>",
        ] {
            assert!(!value.contains(marker), "unexpected marker {marker}");
        }
    }

    #[test]
    fn stream_delta_preserves_raw_assistant_text_as_sse_data() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let writer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            write_stream_delta(
                &mut stream,
                "chatcmpl-test",
                "tablinq-instant",
                "req-1",
                EventPayload::StreamChunk(StreamChunk {
                    job_id: "job-1".to_owned(),
                    conversation_id: Some("conv-1".to_owned()),
                    message_id: Some("msg-1".to_owned()),
                    seq: 1,
                    kind: StreamChunkKind::Delta,
                    text: raw_assistant_text().to_owned(),
                }),
            )
            .unwrap();
        });

        let mut client = TcpStream::connect(addr).unwrap();
        let mut sse_text = String::new();
        client.read_to_string(&mut sse_text).unwrap();
        writer.join().unwrap();

        let data = sse_text
            .strip_prefix("data: ")
            .and_then(|value| value.strip_suffix("\n\n"))
            .expect("SSE data event");
        let event: Value = serde_json::from_str(data).unwrap();

        assert_eq!(
            event["choices"][0]["delta"]["content"].as_str(),
            Some(raw_assistant_text())
        );
        assert_no_cli_display_markers(&sse_text);
    }

    #[test]
    fn stream_error_is_tagged_as_an_error_object() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let writer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            write_stream_error(
                &mut stream,
                &json!({
                    "request_id": "req-1",
                    "error": {
                        "message": "capability missing",
                        "type": "invalid_request_error",
                        "code": "capability_missing",
                    },
                }),
            )
            .unwrap();
        });

        let mut client = TcpStream::connect(addr).unwrap();
        let mut sse_text = String::new();
        client.read_to_string(&mut sse_text).unwrap();
        writer.join().unwrap();

        let data = sse_text
            .strip_prefix("data: ")
            .and_then(|value| value.strip_suffix("\n\n"))
            .expect("SSE data event");
        let event: Value = serde_json::from_str(data).unwrap();

        assert_eq!(event["object"], "error");
        assert_eq!(event["request_id"], "req-1");
        assert_eq!(event["error"]["code"], "capability_missing");
    }
}
