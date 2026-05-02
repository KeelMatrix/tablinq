use crate::validation::{
    ValidationError, validate_exact_version, validate_optional, validate_paths, validate_required,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub fn default_auto_recover_provider_generation_failure() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Normal,
    Search,
    Research,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ModelClass {
    Instant,
    Thinking,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    None,
    Standard,
    Extended,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CommandEnvelope {
    pub version: String,
    pub request_id: String,
    pub session_id: Option<String>,
    pub sent_at: DateTime<Utc>,
    pub command: CommandPayload,
}

impl CommandEnvelope {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_exact_version(&self.version)?;
        validate_required("request_id", &self.request_id)?;
        validate_optional("session_id", self.session_id.as_deref())?;
        self.command.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum CommandPayload {
    Ask(AskCommand),
    Research(ResearchCommand),
    CancelJob(CancelJobCommand),
    InspectCapabilities(InspectCapabilitiesCommand),
    HealthPing(HealthPingCommand),
    PullArtifact(PullArtifactCommand),
}

impl CommandPayload {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Ask(command) => command.validate(),
            Self::Research(command) => command.validate(),
            Self::CancelJob(command) => command.validate(),
            Self::InspectCapabilities(command) => command.validate(),
            Self::HealthPing(command) => command.validate(),
            Self::PullArtifact(command) => command.validate(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AskCommand {
    pub prompt: String,
    pub conversation_id: Option<String>,
    pub new_conversation: bool,
    pub mode: Mode,
    pub model_class: Option<ModelClass>,
    pub effort: Option<Effort>,
    pub files: Vec<String>,
    #[serde(default = "default_auto_recover_provider_generation_failure")]
    pub auto_recover_provider_generation_failure: bool,
}

impl AskCommand {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("prompt", &self.prompt)?;
        validate_optional("conversation_id", self.conversation_id.as_deref())?;
        if self.new_conversation && self.conversation_id.is_some() {
            return Err(ValidationError::InvalidFieldCombination {
                field: "conversation_id".to_owned(),
                message: "conversation_id cannot be combined with new_conversation".to_owned(),
            });
        }
        if self.mode == Mode::Research {
            return Err(ValidationError::InvalidFieldCombination {
                field: "mode".to_owned(),
                message: "research must use the dedicated research command".to_owned(),
            });
        }
        validate_paths("files", &self.files)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResearchCommand {
    pub query: String,
    pub conversation_id: Option<String>,
    pub new_conversation: bool,
    pub files: Vec<String>,
    pub output_dir: Option<String>,
}

impl ResearchCommand {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("query", &self.query)?;
        validate_optional("conversation_id", self.conversation_id.as_deref())?;
        if self.new_conversation && self.conversation_id.is_some() {
            return Err(ValidationError::InvalidFieldCombination {
                field: "conversation_id".to_owned(),
                message: "conversation_id cannot be combined with new_conversation".to_owned(),
            });
        }
        validate_optional("output_dir", self.output_dir.as_deref())?;
        validate_paths("files", &self.files)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSelectCommand {
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub new_conversation: bool,
}

impl ConversationSelectCommand {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_optional("conversation_id", self.conversation_id.as_deref())?;
        match (self.conversation_id.is_some(), self.new_conversation) {
            (true, false) | (false, true) => Ok(()),
            (true, true) => Err(ValidationError::InvalidFieldCombination {
                field: "conversation_id".to_owned(),
                message: "conversation_id cannot be combined with new_conversation".to_owned(),
            }),
            (false, false) => Err(ValidationError::InvalidFieldCombination {
                field: "conversation_id".to_owned(),
                message: "conversation_id or new_conversation is required".to_owned(),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CancelJobCommand {
    pub job_id: String,
}

impl CancelJobCommand {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("job_id", &self.job_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct InspectCapabilitiesCommand {}

impl InspectCapabilitiesCommand {
    pub fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct HealthPingCommand {}

impl HealthPingCommand {
    pub fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PullArtifactCommand {
    pub artifact_id: String,
    pub destination_dir: String,
}

impl PullArtifactCommand {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("artifact_id", &self.artifact_id)?;
        validate_required("destination_dir", &self.destination_dir)
    }
}
