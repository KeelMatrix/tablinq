use std::collections::BTreeMap;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shared_protocol::{CapabilityReport, Effort, Mode, ModelClass};

#[cfg(test)]
use chrono::Utc;
#[cfg(test)]
use shared_protocol::{CapabilityMatrix, RuntimeAliases};

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
    #[serde(default)]
    pub file_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlattenedMessages {
    pub prompt: String,
    pub file_paths: Vec<String>,
    pub file_ids: Vec<String>,
}

impl ChatCompletionsRequest {
    pub fn request_role(&self) -> RequestRole {
        self.tablinq_request_role.unwrap_or(RequestRole::Primary)
    }
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

pub fn flatten_messages(request: &ChatCompletionsRequest) -> Result<FlattenedMessages> {
    let mut sections = Vec::new();
    let mut file_paths = request.tablinq_files.clone();
    let mut file_ids = request.tablinq_file_ids.clone();

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
                            if let Some(text) = part.text.as_ref()
                                && !text.trim().is_empty()
                            {
                                text_parts.push(text.trim().to_owned());
                            }
                        }
                        "input_text" => {
                            if let Some(text) = part.input_text.as_ref()
                                && !text.trim().is_empty()
                            {
                                text_parts.push(text.trim().to_owned());
                            }
                        }
                        "input_file" => {
                            if let Some(file_id) = part.file_id.as_ref() {
                                if file_id.trim().is_empty() {
                                    bail!("input_file file_id must not be empty");
                                }
                                file_ids.push(file_id.trim().to_owned());
                            } else if let Some(file_path) = part.file_path.as_ref() {
                                if file_path.trim().is_empty() {
                                    bail!("input_file file_path must not be empty");
                                }
                                file_paths.push(file_path.trim().to_owned());
                            } else {
                                bail!("input_file parts must include file_id or file_path");
                            }
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

    Ok(FlattenedMessages {
        prompt: sections.join("\n\n"),
        file_paths,
        file_ids,
    })
}

pub fn model_object(
    spec: &ProviderModelSpec,
    report: Option<&CapabilityReport>,
    capability_source: &str,
) -> Value {
    let availability = model_availability(spec, report);
    json!({
        "id": spec.id,
        "object": "model",
        "created": 0,
        "owned_by": "tablinq",
        "tablinq_mode": mode_name(spec.mode),
        "tablinq_model_class": spec.model_class.map(model_class_name),
        "tablinq_effort": spec.effort.map(effort_name),
        "tablinq_execution_kind": if spec.uses_research { "research" } else { "ask" },
        "tablinq_streaming": true,
        "tablinq_files": true,
        "tablinq_artifacts": true,
        "tablinq_availability": availability.availability,
        "tablinq_warning": availability.warnings,
        "tablinq_capability_source": capability_source,
        "tablinq_aliases": observed_aliases(spec, report),
    })
}

pub fn model_availability(
    spec: &ProviderModelSpec,
    report: Option<&CapabilityReport>,
) -> CapabilityAvailability {
    let Some(report) = report else {
        return CapabilityAvailability {
            availability: "unknown",
            warnings: vec![
                "live capability availability is unknown until the managed browser reports probes"
                    .to_owned(),
            ],
        };
    };

    let available = match spec.id {
        "tablinq-instant" => composer_submission_ready(report),
        "tablinq-thinking-standard" | "tablinq-thinking-extended" => {
            composer_submission_ready(report)
                && thinking_effort_available(report, spec.effort.unwrap_or(Effort::None))
        }
        "tablinq-search" => search_capability_available(report),
        "tablinq-research" => research_capability_available(report),
        _ => false,
    };
    if available {
        CapabilityAvailability {
            availability: "available",
            warnings: Vec::new(),
        }
    } else {
        CapabilityAvailability {
            availability: "unavailable",
            warnings: vec![
                "the current managed session does not expose the required live capability"
                    .to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub struct CapabilityAvailability {
    pub availability: &'static str,
    pub warnings: Vec<String>,
}

pub fn provider_capability_catalog(
    report: Option<&CapabilityReport>,
    capability_source: &str,
) -> Value {
    let model_ids = PROVIDER_MODELS
        .iter()
        .map(|spec| spec.id)
        .collect::<Vec<_>>();
    let available_model_ids = PROVIDER_MODELS
        .iter()
        .filter(|spec| model_availability(spec, report).availability == "available")
        .map(|spec| spec.id)
        .collect::<Vec<_>>();

    json!({
        "capability_source": capability_source,
        "model_ids": model_ids.clone(),
        "available_model_ids": available_model_ids,
        "capabilities": {
            "search": capability_status(
                report,
                "tablinq-search",
                search_capability_available,
                "search mode is unavailable in the current managed session",
            ),
            "research": capability_status(
                report,
                "tablinq-research",
                research_capability_available,
                "research mode is unavailable in the current managed session",
            ),
            "uploads": capability_status(
                report,
                "all_models",
                |value| value.matrix.upload_control_visible,
                "file upload controls are unavailable in the current managed session",
            ),
            "artifacts": capability_status(
                report,
                "all_models",
                |value| value.matrix.downloadable_artifacts_observable,
                "downloadable artifacts are unavailable in the current managed session",
            ),
            "streaming": json!({
                "availability": "available",
                "stable_model_ids": model_ids,
                "warning": Value::Null,
            }),
        },
        "max_safe_parallelism": {
            "requests": 1,
            "branches": 1,
            "subagents": 0,
        },
        "subagent_recommendation": {
            "policy": "disabled",
            "reason": "single-browser upstream execution is serialized through one managed session",
        },
    })
}

fn capability_status(
    report: Option<&CapabilityReport>,
    stable_model_ids: impl Serialize,
    predicate: impl FnOnce(&CapabilityReport) -> bool,
    unavailable_warning: &str,
) -> Value {
    match report {
        Some(report) if predicate(report) => json!({
            "availability": "available",
            "stable_model_ids": stable_model_ids,
            "warning": Value::Null,
        }),
        Some(_) => json!({
            "availability": "unavailable",
            "stable_model_ids": stable_model_ids,
            "warning": unavailable_warning,
        }),
        None => json!({
            "availability": "unknown",
            "stable_model_ids": stable_model_ids,
            "warning": "live capability availability is unknown until the managed browser reports probes",
        }),
    }
}

fn composer_submission_ready(report: &CapabilityReport) -> bool {
    report.matrix.composer_ready && report.matrix.submit_available
}

fn thinking_effort_available(report: &CapabilityReport, effort: Effort) -> bool {
    report.matrix.thinking_effort_visible || has_effort_alias(report, effort_name(effort))
}

fn search_capability_available(report: &CapabilityReport) -> bool {
    composer_submission_ready(report)
        && (report.matrix.search_mode_visible || has_mode_alias(report, "search"))
}

fn research_capability_available(report: &CapabilityReport) -> bool {
    report.matrix.research_mode_visible || has_mode_alias(report, "research")
}

fn has_mode_alias(report: &CapabilityReport, key: &str) -> bool {
    has_alias_values(&report.aliases.modes, key)
}

fn has_effort_alias(report: &CapabilityReport, key: &str) -> bool {
    has_alias_values(&report.aliases.efforts, key)
}

fn has_alias_values(aliases: &BTreeMap<String, Vec<String>>, key: &str) -> bool {
    aliases
        .get(key)
        .map(|values| values.iter().any(|value| !value.trim().is_empty()))
        .unwrap_or(false)
}

fn observed_aliases(spec: &ProviderModelSpec, report: Option<&CapabilityReport>) -> Value {
    let Some(report) = report else {
        return json!({
            "modes": Vec::<String>::new(),
            "model_classes": Vec::<String>::new(),
            "efforts": Vec::<String>::new(),
        });
    };

    let mode_key = match spec.mode {
        Mode::Normal => "normal",
        Mode::Search => "search",
        Mode::Research => "research",
    };
    let model_class_key = match spec.model_class {
        Some(ModelClass::Instant) => Some("instant"),
        Some(ModelClass::Thinking) => Some("thinking"),
        None => None,
    };
    let effort_key = match spec.effort {
        Some(Effort::None) => Some("none"),
        Some(Effort::Standard) => Some("standard"),
        Some(Effort::Extended) => Some("extended"),
        None => None,
    };

    json!({
        "modes": report.aliases.modes.get(mode_key).cloned().unwrap_or_default(),
        "model_classes": model_class_key
            .and_then(|key| report.aliases.model_classes.get(key).cloned())
            .unwrap_or_default(),
        "efforts": effort_key
            .and_then(|key| report.aliases.efforts.get(key).cloned())
            .unwrap_or_default(),
    })
}

pub fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Normal => "normal",
        Mode::Search => "search",
        Mode::Research => "research",
    }
}

fn model_class_name(value: ModelClass) -> &'static str {
    match value {
        ModelClass::Instant => "instant",
        ModelClass::Thinking => "thinking",
    }
}

fn effort_name(value: Effort) -> &'static str {
    match value {
        Effort::None => "none",
        Effort::Standard => "standard",
        Effort::Extended => "extended",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn report_with_aliases(matrix: CapabilityMatrix, aliases: RuntimeAliases) -> CapabilityReport {
        CapabilityReport {
            reported_at: Utc::now(),
            selector_pack_id: "test-pack".to_owned(),
            selector_pack_version: "v1".to_owned(),
            matrix,
            aliases,
        }
    }

    fn aliases_with_mode(key: &str, alias: &str) -> RuntimeAliases {
        let mut aliases = RuntimeAliases::default();
        aliases
            .modes
            .insert(key.to_owned(), vec![alias.to_owned()]);
        aliases
    }

    fn aliases_with_effort(key: &str, alias: &str) -> RuntimeAliases {
        let mut aliases = RuntimeAliases::default();
        aliases
            .efforts
            .insert(key.to_owned(), vec![alias.to_owned()]);
        aliases
    }

    #[test]
    fn search_and_research_use_aliases_when_matrix_lags() {
        let report = report_with_aliases(
            CapabilityMatrix {
                composer_ready: true,
                submit_available: true,
                ..CapabilityMatrix::default()
            },
            RuntimeAliases {
                modes: BTreeMap::from([
                    ("search".to_owned(), vec!["search".to_owned()]),
                    ("research".to_owned(), vec!["deep research".to_owned()]),
                ]),
                ..RuntimeAliases::default()
            },
        );

        assert_eq!(
            model_availability(&PROVIDER_MODELS[3], Some(&report)).availability,
            "available"
        );
        assert_eq!(
            model_availability(&PROVIDER_MODELS[4], Some(&report)).availability,
            "available"
        );
    }

    #[test]
    fn thinking_models_require_effort_signal() {
        let report = report_with_aliases(
            CapabilityMatrix {
                composer_ready: true,
                submit_available: true,
                ..CapabilityMatrix::default()
            },
            aliases_with_effort("standard", "standard"),
        );

        assert_eq!(
            model_availability(&PROVIDER_MODELS[1], Some(&report)).availability,
            "available"
        );
        assert_eq!(
            model_availability(&PROVIDER_MODELS[2], Some(&report)).availability,
            "unavailable"
        );
    }

    #[test]
    fn provider_catalog_keeps_missing_modes_unavailable_without_aliases() {
        let report = report_with_aliases(
            CapabilityMatrix {
                composer_ready: true,
                submit_available: true,
                ..CapabilityMatrix::default()
            },
            aliases_with_mode("normal", "chat"),
        );

        let provider = provider_capability_catalog(Some(&report), "live");
        assert_eq!(provider["capabilities"]["search"]["availability"], "unavailable");
        assert_eq!(
            provider["capabilities"]["research"]["availability"],
            "unavailable"
        );
    }
}

pub fn request_role_name(role: RequestRole) -> &'static str {
    match role {
        RequestRole::Primary => "primary",
        RequestRole::Subagent => "subagent",
        RequestRole::Branch => "branch",
    }
}
