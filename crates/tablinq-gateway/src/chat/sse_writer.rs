use super::PreparedChatRequest;
use crate::http::RequestContext;
use crate::openai::request_role_name;
use crate::sse::{write_sse_done, write_sse_event, write_sse_headers};
use anyhow::Result;
use chrono::Utc;
use serde_json::{Value, json};
use std::net::TcpStream;

pub(super) fn write_stream_headers(stream: &mut TcpStream, request_id: &str) -> Result<()> {
    write_sse_headers(stream, request_id)
}

pub(super) fn write_stream_content_chunk(
    stream: &mut TcpStream,
    stream_id: &str,
    prepared: &PreparedChatRequest,
    context: &RequestContext,
    conversation_id: Option<&str>,
    session_id: Option<&str>,
    content: &str,
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
            "tablinq_conversation_id": conversation_id,
            "tablinq_session_id": session_id,
            "choices": [{
                "index": 0,
                "delta": { "content": content },
                "finish_reason": Value::Null,
            }],
        }),
    )
}

pub(super) fn write_stream_finish_chunk(
    stream: &mut TcpStream,
    stream_id: &str,
    prepared: &PreparedChatRequest,
    context: &RequestContext,
    conversation_id: Option<&str>,
    session_id: Option<&str>,
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
            "tablinq_conversation_id": conversation_id,
            "tablinq_session_id": session_id,
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
    fn stream_content_chunk_preserves_raw_assistant_text_as_sse_data() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let writer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request: crate::openai::ChatCompletionsRequest = serde_json::from_value(json!({
                "model": "tablinq-instant",
                "messages": [{ "role": "user", "content": "hello" }]
            }))
            .unwrap();
            let prepared = PreparedChatRequest {
                request,
                model_id: "tablinq-instant",
                request_role: crate::openai::RequestRole::Primary,
                parent_request_id: None,
                file_ids: Vec::new(),
                job: super::super::PreparedChatJob::Ask(gptd::core::PreparedAskJob {
                    job_id: "job-1".to_owned(),
                    created_at: chrono::Utc::now(),
                    command: shared_protocol::AskCommand {
                        prompt: "hello".to_owned(),
                        conversation_id: None,
                        new_conversation: false,
                        mode: shared_protocol::Mode::Normal,
                        model_class: Some(shared_protocol::ModelClass::Instant),
                        effort: Some(shared_protocol::Effort::None),
                        files: Vec::new(),
                        prompt_upload_threshold_chars: None,
                        auto_recover_provider_generation_failure: true,
                    },
                }),
            };
            let context = RequestContext {
                request_id: "req-1".to_owned(),
                request: crate::http::HttpRequest {
                    method: "POST".to_owned(),
                    path: "/v1/chat/completions".to_owned(),
                    headers: Default::default(),
                    body: Vec::new(),
                },
            };
            write_stream_content_chunk(
                &mut stream,
                "chatcmpl-test",
                &prepared,
                &context,
                Some("conv-1"),
                Some("browser-primary"),
                raw_assistant_text(),
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
