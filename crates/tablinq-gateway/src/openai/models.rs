use std::collections::BTreeMap;

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

#[derive(Debug, Clone)]
pub struct CapabilityAvailability {
    pub availability: &'static str,
    pub warnings: Vec<String>,
}

pub fn resolve_model(model_id: &str) -> Option<&'static ProviderModelSpec> {
    PROVIDER_MODELS.iter().find(|spec| spec.id == model_id)
}

pub fn model_object(
    spec: &ProviderModelSpec,
    report: Option<&CapabilityReport>,
    capability_source: &str,
) -> Value {
    let availability = model_availability(spec, report);
    let streaming = capability_status(
        report,
        composer_submission_ready,
        "streaming submission is unavailable until the attached browser-backed ChatGPT tab is ready",
    );
    let files = capability_status(
        report,
        |value| composer_submission_ready(value) && value.matrix.upload_control_visible,
        "file upload controls are unavailable in the attached browser-backed ChatGPT tab",
    );
    let artifacts = capability_status(
        report,
        |value| value.matrix.downloadable_artifacts_observable,
        "downloadable artifacts are unavailable in the attached browser-backed ChatGPT tab",
    );
    json!({
        "id": spec.id,
        "object": "model",
        "created": 0,
        "owned_by": "tablinq",
        "tablinq_mode": mode_name(spec.mode),
        "tablinq_model_class": spec.model_class.map(model_class_name),
        "tablinq_effort": spec.effort.map(effort_name),
        "tablinq_execution_kind": if spec.uses_research { "research" } else { "ask" },
        "tablinq_streaming": streaming.availability == "available",
        "tablinq_streaming_status": streaming.availability,
        "tablinq_files": files.availability == "available",
        "tablinq_files_status": files.availability,
        "tablinq_artifacts": artifacts.availability == "available",
        "tablinq_artifacts_status": artifacts.availability,
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
            warnings: vec![unknown_capability_warning()],
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
        let warning = match spec.id {
            "tablinq-instant" => {
                "the attached browser-backed ChatGPT tab is not ready to submit prompts"
            }
            "tablinq-thinking-standard" | "tablinq-thinking-extended" => {
                if !composer_submission_ready(report) {
                    "the attached browser-backed ChatGPT tab is not ready to submit prompts"
                } else {
                    "the attached browser-backed ChatGPT tab does not expose the required thinking effort controls"
                }
            }
            "tablinq-search" => {
                "search mode is unavailable in the attached browser-backed ChatGPT tab"
            }
            "tablinq-research" => {
                "research mode is unavailable or the page is not ready to submit in the attached browser-backed ChatGPT tab"
            }
            _ => {
                "the attached browser-backed ChatGPT tab does not currently expose the required capability"
            }
        };
        CapabilityAvailability {
            availability: "unavailable",
            warnings: vec![warning.to_owned()],
        }
    }
}

pub(super) fn composer_submission_ready(report: &CapabilityReport) -> bool {
    report.matrix.composer_ready && report.matrix.submit_available
}

pub(super) fn search_capability_available(report: &CapabilityReport) -> bool {
    composer_submission_ready(report)
        && (report.matrix.search_mode_visible || has_mode_alias(report, "search"))
}

pub(super) fn research_capability_available(report: &CapabilityReport) -> bool {
    composer_submission_ready(report)
        && (report.matrix.research_mode_visible || has_mode_alias(report, "research"))
}

fn thinking_effort_available(report: &CapabilityReport, effort: Effort) -> bool {
    report.matrix.thinking_effort_visible || has_effort_alias(report, effort_name(effort))
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

fn capability_status(
    report: Option<&CapabilityReport>,
    predicate: impl FnOnce(&CapabilityReport) -> bool,
    unavailable_warning: &str,
) -> CapabilityAvailability {
    match report {
        Some(report) if predicate(report) => CapabilityAvailability {
            availability: "available",
            warnings: Vec::new(),
        },
        Some(_) => CapabilityAvailability {
            availability: "unavailable",
            warnings: vec![unavailable_warning.to_owned()],
        },
        None => CapabilityAvailability {
            availability: "unknown",
            warnings: vec![unknown_capability_warning()],
        },
    }
}

fn unknown_capability_warning() -> String {
    "live capability availability is unknown until the attached browser-backed ChatGPT tab reports probes"
        .to_owned()
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

pub(super) fn effort_name(value: Effort) -> &'static str {
    match value {
        Effort::None => "none",
        Effort::Standard => "standard",
        Effort::Extended => "extended",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use shared_protocol::{CapabilityMatrix, RuntimeAliases};

    fn report_with_aliases(matrix: CapabilityMatrix, aliases: RuntimeAliases) -> CapabilityReport {
        CapabilityReport {
            reported_at: Utc::now(),
            selector_pack_id: "test-pack".to_owned(),
            selector_pack_version: "v1".to_owned(),
            matrix,
            aliases,
        }
    }

    fn aliases_with_effort(key: &str, alias: &str) -> RuntimeAliases {
        let mut aliases = RuntimeAliases::default();
        aliases
            .efforts
            .insert(key.to_owned(), vec![alias.to_owned()]);
        aliases
    }

    #[test]
    fn model_availability_uses_live_capability_report_aliases() {
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
    fn research_model_requires_submit_ready_page_state() {
        let report = report_with_aliases(
            CapabilityMatrix {
                composer_ready: false,
                submit_available: false,
                research_mode_visible: true,
                ..CapabilityMatrix::default()
            },
            RuntimeAliases {
                modes: BTreeMap::from([("research".to_owned(), vec!["deep research".to_owned()])]),
                ..RuntimeAliases::default()
            },
        );

        assert_eq!(
            model_availability(&PROVIDER_MODELS[4], Some(&report)).availability,
            "unavailable"
        );
    }

    #[test]
    fn model_object_reflects_live_optional_capability_support() {
        let report = report_with_aliases(
            CapabilityMatrix {
                composer_ready: true,
                submit_available: true,
                upload_control_visible: false,
                downloadable_artifacts_observable: false,
                ..CapabilityMatrix::default()
            },
            RuntimeAliases::default(),
        );

        let model = model_object(&PROVIDER_MODELS[0], Some(&report), "live");

        assert_eq!(model["tablinq_streaming"], true);
        assert_eq!(model["tablinq_streaming_status"], "available");
        assert_eq!(model["tablinq_files"], false);
        assert_eq!(model["tablinq_files_status"], "unavailable");
        assert_eq!(model["tablinq_artifacts"], false);
        assert_eq!(model["tablinq_artifacts_status"], "unavailable");
    }
}
