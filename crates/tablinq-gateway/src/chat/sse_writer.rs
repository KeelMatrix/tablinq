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
    write_sse_event(stream, error)
}

pub(super) fn write_stream_done(stream: &mut TcpStream) -> Result<()> {
    write_sse_done(stream)
}
