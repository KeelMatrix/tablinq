use crate::validation::{ValidationError, validate_required};
use crate::{
    ConfigSnapshot, DaemonStatusSnapshot, FailureBundle, HealthCheck, SELECTOR_PACK_FORMAT_VERSION,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrowserConnectionMode {
    #[default]
    AttachFirst,
    ManagedFallback,
    ManagedOnly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum BrowserOwner {
    External,
    #[default]
    Managed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthSource {
    Page,
    Extension,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    AriaRole,
    Css,
    Text,
    DomNeighborhood,
    Hotkey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct CapabilityMatrix {
    pub composer_ready: bool,
    pub submit_available: bool,
    pub model_picker_visible: bool,
    pub thinking_effort_visible: bool,
    pub search_mode_visible: bool,
    pub research_mode_visible: bool,
    pub upload_control_visible: bool,
    pub downloadable_artifacts_observable: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityObservationStatus {
    Observed,
    NotObserved,
    #[default]
    Unknown,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct BinaryCapabilityObservation {
    #[serde(default)]
    pub status: CapabilityObservationStatus,
    #[serde(default)]
    pub observed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ModelSelectionObservation {
    #[serde(default)]
    pub status: CapabilityObservationStatus,
    #[serde(default)]
    pub picker_visible: bool,
    #[serde(default)]
    pub selected_model_class: Option<String>,
    #[serde(default)]
    pub selected_label: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub unsupported_pro_selected: bool,
    #[serde(default)]
    pub ambiguous_selection: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ThinkingEffortObservation {
    #[serde(default)]
    pub status: CapabilityObservationStatus,
    #[serde(default)]
    pub controls_visible: bool,
    #[serde(default)]
    pub selected_effort: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct SearchModeObservation {
    #[serde(default)]
    pub status: CapabilityObservationStatus,
    #[serde(default)]
    pub visible: bool,
    #[serde(default)]
    pub selected: bool,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct ResearchFlowObservation {
    #[serde(default)]
    pub status: CapabilityObservationStatus,
    #[serde(default)]
    pub entrypoint_visible: bool,
    #[serde(default)]
    pub on_research_url: bool,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct CapabilityObservedUi {
    #[serde(default)]
    pub composer_readiness: BinaryCapabilityObservation,
    #[serde(default)]
    pub submit_availability: BinaryCapabilityObservation,
    #[serde(default)]
    pub selected_model: ModelSelectionObservation,
    #[serde(default)]
    pub thinking_effort: ThinkingEffortObservation,
    #[serde(default)]
    pub search_mode: SearchModeObservation,
    #[serde(default)]
    pub research_flow: ResearchFlowObservation,
    #[serde(default)]
    pub upload_availability: BinaryCapabilityObservation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CapabilityReport {
    pub reported_at: DateTime<Utc>,
    pub selector_pack_id: String,
    pub selector_pack_version: String,
    pub matrix: CapabilityMatrix,
    pub aliases: RuntimeAliases,
    #[serde(default)]
    pub observed_ui: CapabilityObservedUi,
}

impl CapabilityReport {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("selector_pack_id", &self.selector_pack_id)?;
        validate_required("selector_pack_version", &self.selector_pack_version)?;
        self.aliases.validate()?;
        self.observed_ui.validate()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct RuntimeAliases {
    pub modes: BTreeMap<String, Vec<String>>,
    pub model_classes: BTreeMap<String, Vec<String>>,
    pub efforts: BTreeMap<String, Vec<String>>,
}

impl RuntimeAliases {
    pub fn validate(&self) -> Result<(), ValidationError> {
        for (field, groups) in [
            ("aliases.modes", &self.modes),
            ("aliases.model_classes", &self.model_classes),
            ("aliases.efforts", &self.efforts),
        ] {
            for (name, labels) in groups {
                validate_required(field, name)?;
                for label in labels {
                    validate_required(field, label)?;
                }
            }
        }
        Ok(())
    }
}

impl CapabilityObservedUi {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_optional_observation_text(
            "observed_ui.selected_model.selected_model_class",
            self.selected_model.selected_model_class.as_deref(),
        )?;
        validate_optional_observation_text(
            "observed_ui.selected_model.selected_label",
            self.selected_model.selected_label.as_deref(),
        )?;
        validate_optional_observation_text(
            "observed_ui.thinking_effort.selected_effort",
            self.thinking_effort.selected_effort.as_deref(),
        )?;
        validate_labels(
            "observed_ui.selected_model.labels",
            &self.selected_model.labels,
        )?;
        validate_labels(
            "observed_ui.thinking_effort.labels",
            &self.thinking_effort.labels,
        )?;
        validate_labels("observed_ui.search_mode.labels", &self.search_mode.labels)?;
        validate_labels(
            "observed_ui.research_flow.labels",
            &self.research_flow.labels,
        )
    }
}

fn validate_optional_observation_text(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), ValidationError> {
    if let Some(value) = value {
        validate_required(field, value)?;
    }
    Ok(())
}

fn validate_labels(field: &'static str, labels: &[String]) -> Result<(), ValidationError> {
    for label in labels {
        validate_required(field, label)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SelectorPack {
    pub format_version: u32,
    pub pack_id: String,
    pub description: String,
    pub updated_at: DateTime<Utc>,
    pub capabilities: BTreeMap<String, Vec<SelectorStrategy>>,
}

impl SelectorPack {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.format_version != SELECTOR_PACK_FORMAT_VERSION {
            return Err(ValidationError::UnsupportedSelectorPackFormat {
                found: self.format_version,
                expected: SELECTOR_PACK_FORMAT_VERSION,
            });
        }

        validate_required("pack_id", &self.pack_id)?;
        validate_required("description", &self.description)?;

        if self.capabilities.is_empty() {
            return Err(ValidationError::EmptyCollection("capabilities"));
        }

        for (capability, strategies) in &self.capabilities {
            validate_required("capabilities", capability)?;
            if strategies.is_empty() {
                return Err(ValidationError::EmptyCollection("capability strategies"));
            }

            let mut seen = BTreeSet::new();
            for strategy in strategies {
                strategy.validate()?;
                if !seen.insert(strategy.strategy_id.clone()) {
                    return Err(ValidationError::DuplicateStrategy {
                        field: capability.clone(),
                        strategy_id: strategy.strategy_id.clone(),
                    });
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SelectorStrategy {
    pub strategy_id: String,
    pub kind: StrategyKind,
    pub selector: String,
    pub score: f32,
}

impl SelectorStrategy {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_required("strategy_id", &self.strategy_id)?;
        validate_required("selector", &self.selector)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SelectorPackSummary {
    pub pack_id: String,
    pub version: String,
    pub updated_at: Option<DateTime<Utc>>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StrategyTelemetryEntry {
    pub capability: String,
    pub strategy_id: String,
    pub success_count: u64,
    pub failure_count: u64,
    pub success_rate: f32,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BrowserDoctor {
    pub browser_detected: bool,
    pub detected_browser_path: Option<String>,
    pub profile_accessible: bool,
    pub profile_access_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DoctorReport {
    pub health: HealthCheck,
    pub status: DaemonStatusSnapshot,
    pub config: ConfigSnapshot,
    pub browser: BrowserStatusSnapshot,
    pub capabilities: Option<CapabilityReport>,
    pub selector_pack: Option<SelectorPackSummary>,
    pub strategy_telemetry: Vec<StrategyTelemetryEntry>,
    pub failure_bundle_count: u64,
    pub latest_failure_bundle: Option<FailureBundle>,
    pub browser_detection: BrowserDoctor,
    pub recovery_guidance: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CapabilitySnapshot {
    pub report: Option<CapabilityReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BrowserOpenRequest {
    pub target_url: Option<String>,
    pub browser_path: Option<String>,
    pub headless: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BrowserStatusSnapshot {
    pub running: bool,
    pub browser_kind: String,
    pub profile_dir: String,
    #[serde(default)]
    pub attach_profile_dir: Option<String>,
    #[serde(default)]
    pub attach_profile_note: Option<String>,
    pub downloads_dir: String,
    pub target_url: String,
    pub current_url: Option<String>,
    pub page_title: Option<String>,
    pub page_ready: bool,
    pub authenticated: bool,
    pub reload_count: u64,
    pub screenshot_path: Option<String>,
    pub browser_path: Option<String>,
    pub extension_connected: bool,
    pub capability_report: Option<CapabilityReport>,
    #[serde(default)]
    pub attach_mode: BrowserConnectionMode,
    #[serde(default)]
    pub browser_owner: BrowserOwner,
    #[serde(default)]
    pub cdp_attached: bool,
    #[serde(default)]
    pub target_tab_found: bool,
    #[serde(default)]
    pub target_tab_id: Option<String>,
    #[serde(default)]
    pub target_workspace_key: Option<String>,
    #[serde(default)]
    pub debug_endpoint: Option<String>,
    #[serde(default)]
    pub auth_source: AuthSource,
    #[serde(default)]
    pub extension_required: bool,
    #[serde(default)]
    pub last_capability_report_at: Option<String>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub browser_process_closed: Option<bool>,
    #[serde(default)]
    pub reason: Option<String>,
}
