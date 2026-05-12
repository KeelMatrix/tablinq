use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::models::resolve_model;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiRequestError {
    pub status: u16,
    pub error_type: &'static str,
    pub code: &'static str,
    pub message: String,
}

impl OpenAiRequestError {
    pub fn parts(&self) -> (u16, &'static str, &'static str, String) {
        (
            self.status,
            self.error_type,
            self.code,
            self.message.clone(),
        )
    }
}

impl std::fmt::Display for OpenAiRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for OpenAiRequestError {}

pub fn invalid_request_error(code: &'static str, message: impl Into<String>) -> OpenAiRequestError {
    OpenAiRequestError {
        status: 400,
        error_type: "invalid_request_error",
        code,
        message: message.into(),
    }
}

pub fn invalid_request_json_error(message: impl Into<String>) -> OpenAiRequestError {
    invalid_request_error("invalid_request_json", message)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatCompletionsRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(rename = "temperature", default)]
    pub _temperature: Option<Value>,
    #[serde(rename = "top_p", default)]
    pub _top_p: Option<Value>,
    #[serde(rename = "max_tokens", default)]
    pub _max_tokens: Option<Value>,
    #[serde(rename = "max_completion_tokens", default)]
    pub _max_completion_tokens: Option<Value>,
    #[serde(rename = "stop", default)]
    pub _stop: Option<Value>,
    #[serde(rename = "presence_penalty", default)]
    pub _presence_penalty: Option<Value>,
    #[serde(rename = "frequency_penalty", default)]
    pub _frequency_penalty: Option<Value>,
    #[serde(rename = "user", default)]
    pub _user: Option<Value>,
    #[serde(rename = "metadata", default)]
    pub _metadata: Option<Value>,
    #[serde(rename = "store", default)]
    pub _store: Option<Value>,
    #[serde(rename = "logprobs", default)]
    pub _logprobs: Option<Value>,
    #[serde(rename = "stream_options", default)]
    pub _stream_options: Option<Value>,
    #[serde(default)]
    pub tools: Option<Value>,
    #[serde(rename = "tool_choice", default)]
    pub _tool_choice: Option<Value>,
    #[serde(rename = "parallel_tool_calls", default)]
    pub _parallel_tool_calls: Option<Value>,
    #[serde(default)]
    pub response_format: Option<Value>,
    #[serde(default)]
    pub n: Option<u32>,
    #[serde(default)]
    pub tablinq_files: Vec<String>,
    #[serde(default)]
    pub tablinq_file_ids: Vec<String>,
    #[serde(default)]
    pub tablinq_request_role: Option<RequestRole>,
    #[serde(default)]
    pub tablinq_parent_request_id: Option<String>,
    #[serde(default)]
    pub tablinq_conversation_id: Option<String>,
    #[serde(default)]
    pub tablinq_new_conversation: bool,
    #[serde(default = "shared_protocol::default_auto_recover_provider_generation_failure")]
    pub tablinq_auto_recover_provider_generation_failure: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequestRole {
    Primary,
    Subagent,
    Branch,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    #[serde(rename = "role")]
    pub role: String,
    #[serde(default)]
    pub content: Option<MessageContent>,
    #[serde(rename = "tool_calls", default)]
    pub tool_calls: Option<Value>,
    #[serde(rename = "tool_call_id", default)]
    pub tool_call_id: Option<String>,
    #[serde(rename = "name", default)]
    pub _name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Parts(Vec<MessagePart>),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagePart {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub input_text: Option<String>,
    #[serde(default)]
    pub file_path: Option<String>,
    #[serde(default)]
    pub file_id: Option<String>,
}

impl ChatCompletionsRequest {
    pub fn request_role(&self) -> RequestRole {
        self.tablinq_request_role.unwrap_or(RequestRole::Primary)
    }
}

pub fn validate_chat_request(request: &ChatCompletionsRequest) -> Result<()> {
    if request.messages.is_empty() {
        return Err(invalid_request_error(
            "empty_messages",
            "messages must contain at least one item",
        )
        .into());
    }
    if let Some(tools) = request.tools.as_ref() {
        let unsupported = match tools {
            Value::Array(items) => !items.is_empty(),
            Value::Null => false,
            _ => true,
        };
        if unsupported {
            return Err(invalid_request_error(
                "unsupported_tools",
                "tool calling is not supported by the browser-backed provider",
            )
            .into());
        }
    }
    if let Some(response_format) = request.response_format.as_ref() {
        let unsupported = response_format
            .get("type")
            .and_then(Value::as_str)
            .map(|value| value != "text")
            .unwrap_or(!response_format.is_null());
        if unsupported {
            return Err(invalid_request_error(
                "unsupported_response_format",
                "structured response formats are not supported by the browser-backed provider",
            )
            .into());
        }
    }
    if request.n.unwrap_or(1) != 1 {
        return Err(invalid_request_error(
            "unsupported_n",
            "n>1 is not supported by the browser-backed provider",
        )
        .into());
    }
    if let Some(parent_request_id) = request.tablinq_parent_request_id.as_deref()
        && parent_request_id.trim().is_empty()
    {
        return Err(invalid_request_error(
            "invalid_parent_request_id",
            "tablinq_parent_request_id must not be empty when provided",
        )
        .into());
    }
    if let Some(conversation_id) = request.tablinq_conversation_id.as_deref()
        && conversation_id.trim().is_empty()
    {
        return Err(invalid_request_error(
            "invalid_conversation_id",
            "tablinq_conversation_id must not be empty when provided",
        )
        .into());
    }
    if request.tablinq_new_conversation && request.tablinq_conversation_id.is_some() {
        return Err(invalid_request_error(
            "conflicting_conversation_target",
            "tablinq_conversation_id cannot be combined with tablinq_new_conversation",
        )
        .into());
    }
    if request
        .tablinq_file_ids
        .iter()
        .any(|file_id| file_id.trim().is_empty())
    {
        return Err(invalid_request_error(
            "invalid_file_reference",
            "tablinq_file_ids must not contain empty values",
        )
        .into());
    }
    if request
        .tablinq_files
        .iter()
        .any(|file_path| file_path.trim().is_empty())
    {
        return Err(invalid_request_error(
            "invalid_file_reference",
            "tablinq_files must not contain empty values",
        )
        .into());
    }
    if resolve_model(&request.model).is_none() {
        return Err(invalid_request_error(
            "unsupported_model",
            format!("model {} is not supported", request.model),
        )
        .into());
    }
    for message in &request.messages {
        if message.tool_calls.is_some()
            || message
                .tool_call_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
        {
            return Err(invalid_request_error(
                "unsupported_tool_calls",
                "message tool_calls and tool_call_id fields are not supported by the browser-backed provider",
            )
            .into());
        }
        if message.role.trim() != "user" {
            return Err(invalid_request_error(
                "unsupported_role",
                format!(
                    "message role {} is not supported; only user messages are accepted for browser-backed submission",
                    message.role
                ),
            )
            .into());
        }
        if message.content.is_none() {
            return Err(invalid_request_error(
                "invalid_message_content",
                "user messages must include text content",
            )
            .into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn common_openai_chat_fields_are_tolerated() {
        let request: ChatCompletionsRequest = serde_json::from_value(json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "hello" }],
            "temperature": 0.2,
            "top_p": 0.9,
            "max_tokens": 128,
            "max_completion_tokens": 64,
            "stop": ["END"],
            "presence_penalty": 0.0,
            "frequency_penalty": 0.0,
            "user": "client-user",
            "metadata": { "client": "integration-test" },
            "store": false,
            "logprobs": true,
            "stream_options": { "include_usage": true },
            "tool_choice": "auto",
            "parallel_tool_calls": true
        }))
        .expect("request decodes");

        validate_chat_request(&request).expect("request validates");
    }

    #[test]
    fn unknown_model_is_rejected() {
        let request: ChatCompletionsRequest = serde_json::from_value(json!({
            "model": "tablinq-missing",
            "messages": [{ "role": "user", "content": "hello" }]
        }))
        .expect("request decodes");

        let err = validate_chat_request(&request).expect_err("unknown model should fail");
        assert!(err.to_string().contains("is not supported"));
    }

    #[test]
    fn stream_true_is_accepted_for_final_only_sse() {
        let request: ChatCompletionsRequest = serde_json::from_value(json!({
            "model": "tablinq-instant",
            "stream": true,
            "messages": [{ "role": "user", "content": "hello" }]
        }))
        .expect("request decodes");

        validate_chat_request(&request).expect("streaming request validates");
    }

    #[test]
    fn omitted_stream_defaults_to_false() {
        let request: ChatCompletionsRequest = serde_json::from_value(json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "hello" }]
        }))
        .expect("request decodes");

        assert!(!request.stream);
        validate_chat_request(&request).expect("request validates");
    }

    #[test]
    fn unsupported_tools_are_rejected() {
        let request: ChatCompletionsRequest = serde_json::from_value(json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "hello" }],
            "tools": [{ "type": "function", "function": { "name": "demo" } }]
        }))
        .expect("request decodes");

        let err = validate_chat_request(&request).expect_err("tools should fail");
        assert!(err.to_string().contains("tool calling"));
    }

    #[test]
    fn unsupported_response_format_is_rejected() {
        let request: ChatCompletionsRequest = serde_json::from_value(json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "hello" }],
            "response_format": { "type": "json_schema" }
        }))
        .expect("request decodes");

        let err = validate_chat_request(&request).expect_err("response_format should fail");
        assert!(err.to_string().contains("structured response formats"));
    }

    #[test]
    fn n_greater_than_one_is_rejected() {
        let request: ChatCompletionsRequest = serde_json::from_value(json!({
            "model": "tablinq-instant",
            "messages": [{ "role": "user", "content": "hello" }],
            "n": 2
        }))
        .expect("request decodes");

        let err = validate_chat_request(&request).expect_err("n>1 should fail");
        assert!(err.to_string().contains("n>1"));
    }

    #[test]
    fn non_user_message_roles_are_rejected() {
        for role in ["system", "developer", "assistant", "tool", "custom_role"] {
            let request: ChatCompletionsRequest = serde_json::from_value(json!({
                "model": "tablinq-instant",
                "messages": [{ "role": role, "content": "hello" }]
            }))
            .expect("request decodes");

            let err = validate_chat_request(&request).expect_err("non-user role should fail");
            assert!(err.to_string().contains("only user messages"));
        }
    }

    #[test]
    fn message_tool_calls_are_rejected() {
        let request: ChatCompletionsRequest = serde_json::from_value(json!({
            "model": "tablinq-instant",
            "messages": [{
                "role": "user",
                "content": "hello",
                "tool_calls": [{ "id": "call-1" }]
            }]
        }))
        .expect("request decodes");

        let err = validate_chat_request(&request).expect_err("tool calls should fail");
        assert!(err.to_string().contains("tool_calls"));
    }
}
