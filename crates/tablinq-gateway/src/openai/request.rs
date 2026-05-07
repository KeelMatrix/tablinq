use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::models::resolve_model;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatCompletionsRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub tools: Option<Value>,
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
    pub _role: String,
    pub content: MessageContent,
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
        bail!("messages must contain at least one item");
    }
    if let Some(tools) = request.tools.as_ref() {
        let unsupported = match tools {
            Value::Array(items) => !items.is_empty(),
            Value::Null => false,
            _ => true,
        };
        if unsupported {
            bail!("tool calling is not supported by the browser-backed provider");
        }
    }
    if let Some(response_format) = request.response_format.as_ref() {
        let unsupported = response_format
            .get("type")
            .and_then(Value::as_str)
            .map(|value| value != "text")
            .unwrap_or(!response_format.is_null());
        if unsupported {
            bail!("structured response formats are not supported by the browser-backed provider");
        }
    }
    if request.n.unwrap_or(1) != 1 {
        bail!("n>1 is not supported by the browser-backed provider");
    }
    if let Some(parent_request_id) = request.tablinq_parent_request_id.as_deref()
        && parent_request_id.trim().is_empty()
    {
        bail!("tablinq_parent_request_id must not be empty when provided");
    }
    if let Some(conversation_id) = request.tablinq_conversation_id.as_deref()
        && conversation_id.trim().is_empty()
    {
        bail!("tablinq_conversation_id must not be empty when provided");
    }
    if request.tablinq_new_conversation && request.tablinq_conversation_id.is_some() {
        bail!("tablinq_conversation_id cannot be combined with tablinq_new_conversation");
    }
    if resolve_model(&request.model).is_none() {
        bail!("model {} is not supported", request.model);
    }
    Ok(())
}
