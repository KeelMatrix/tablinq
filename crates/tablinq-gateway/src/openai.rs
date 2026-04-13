use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};
use shared_protocol::{CapabilityReport, Effort, Mode, ModelClass};

#[derive(Debug, Clone, Copy)]
pub struct ProviderModelSpec {
    pub id: &'static str,
    pub mode: Mode,
    pub model_class: Option<ModelClass>,
    pub effort: Option<Effort>,
    pub uses_research: bool,
}

pub const PROVIDER_MODELS: &[ProviderModelSpec] = &[
    ProviderModelSpec {
        id: "tablinq-instant",
        mode: Mode::Normal,
        model_class: Some(ModelClass::Instant),
        effort: Some(Effort::None),
        uses_research: false,
    },
    ProviderModelSpec {
        id: "tablinq-thinking-standard",
        mode: Mode::Normal,
        model_class: Some(ModelClass::Thinking),
        effort: Some(Effort::Standard),
        uses_research: false,
    },
    ProviderModelSpec {
        id: "tablinq-thinking-extended",
        mode: Mode::Normal,
        model_class: Some(ModelClass::Thinking),
        effort: Some(Effort::Extended),
        uses_research: false,
    },
    ProviderModelSpec {
        id: "tablinq-search",
        mode: Mode::Search,
        model_class: None,
        effort: None,
        uses_research: false,
    },
    ProviderModelSpec {
        id: "tablinq-research",
        mode: Mode::Research,
        model_class: None,
        effort: None,
        uses_research: true,
    },
];

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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatMessage {
    pub role: String,
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
}

pub fn resolve_model(model_id: &str) -> Option<&'static ProviderModelSpec> {
    PROVIDER_MODELS.iter().find(|spec| spec.id == model_id)
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
    if resolve_model(&request.model).is_none() {
        bail!("model {} is not supported", request.model);
    }
    Ok(())
}

pub fn flatten_messages(request: &ChatCompletionsRequest) -> Result<(String, Vec<String>)> {
    let mut sections = Vec::new();
    let mut files = request.tablinq_files.clone();

    for message in &request.messages {
        let mut text_parts = Vec::new();
        match &message.content {
            MessageContent::Text(text) => {
                if !text.trim().is_empty() {
                    text_parts.push(text.trim().to_owned());
                }
            }
            MessageContent::Parts(parts) => {
                for part in parts {
                    match part.kind.as_str() {
                        "text" => {
                            if let Some(text) = part.text.as_ref() {
                                if !text.trim().is_empty() {
                                    text_parts.push(text.trim().to_owned());
                                }
                            }
                        }
                        "input_text" => {
                            if let Some(text) = part.input_text.as_ref() {
                                if !text.trim().is_empty() {
                                    text_parts.push(text.trim().to_owned());
                                }
                            }
                        }
                        "input_file" => {
                            let Some(file_path) = part.file_path.as_ref() else {
                                bail!("input_file parts must include file_path");
                            };
                            if file_path.trim().is_empty() {
                                bail!("input_file file_path must not be empty");
                            }
                            files.push(file_path.trim().to_owned());
                        }
                        other => bail!("message content part type {other} is not supported"),
                    }
                }
            }
        }

        if !text_parts.is_empty() {
            sections.push(format!(
                "{}:\n{}",
                message.role.to_uppercase(),
                text_parts.join("\n")
            ));
        }
    }

    if sections.is_empty() {
        bail!("messages do not contain any supported text content");
    }

    Ok((sections.join("\n\n"), files))
}

pub fn model_object(spec: &ProviderModelSpec, report: Option<&CapabilityReport>) -> Value {
    let (availability, warnings) = model_availability(spec, report);
    json!({
        "id": spec.id,
        "object": "model",
        "created": 0,
        "owned_by": "tablinq",
        "tablinq_mode": mode_name(spec.mode),
        "tablinq_availability": availability,
        "tablinq_warning": warnings,
    })
}

fn model_availability(spec: &ProviderModelSpec, report: Option<&CapabilityReport>) -> (&'static str, Vec<String>) {
    let Some(report) = report else {
        return (
            "unknown",
            vec!["live capability availability is unknown until the managed browser reports probes".to_owned()],
        );
    };

    let matrix = &report.matrix;
    let available = match spec.id {
        "tablinq-instant" => matrix.composer_ready && matrix.submit_available,
        "tablinq-thinking-standard" | "tablinq-thinking-extended" => {
            matrix.composer_ready && matrix.submit_available && matrix.thinking_effort_visible
        }
        "tablinq-search" => matrix.composer_ready && matrix.submit_available && matrix.search_mode_visible,
        "tablinq-research" => matrix.research_mode_visible,
        _ => false,
    };
    if available {
        ("available", Vec::new())
    } else {
        (
            "unavailable",
            vec!["the current managed session does not expose the required live capability".to_owned()],
        )
    }
}

pub fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "normal",
        Mode::Search => "search",
        Mode::Research => "research",
    }
}

pub fn openai_error(status: u16, error_type: &str, code: &str, message: &str) -> Value {
    json!({
        "error": {
            "message": message,
            "type": error_type,
            "code": code,
        },
        "status": status,
    })
}
