use std::collections::BTreeMap;

use serde_json::{Value, json};
use shared_protocol::{CapabilityReport, Effort, ModelClass};

#[derive(Debug, Clone, Copy)]
pub struct ProviderModelSpec {
    pub id: &'static str,
    pub model_class: ModelClass,
    pub effort: Option<Effort>,
}

pub const LEGACY_SEARCH_MODEL_ID: &str = "tablinq-search";
pub const RESEARCH_PRESET_ID: &str = "tablinq-research";

pub const PROVIDER_MODELS: &[ProviderModelSpec] = &[
    ProviderModelSpec {
        id: "tablinq-instant",
        model_class: ModelClass::Instant,
        effort: None,
    },
    ProviderModelSpec {
        id: "tablinq-thinking-standard",
        model_class: ModelClass::Thinking,
        effort: Some(Effort::Standard),
    },
    ProviderModelSpec {
        id: "tablinq-thinking-extended",
        model_class: ModelClass::Thinking,
        effort: Some(Effort::Extended),
    },
];

#[derive(Debug, Clone, Copy)]
pub enum ProviderRequestTarget {
    Ask(&'static ProviderModelSpec),
    Research,
}

#[derive(Debug, Clone)]
pub struct CapabilitySnapshotStatus {
    pub snapshot_status: &'static str,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone)]
struct CapabilityAvailability {
    availability: &'static str,
}

pub fn resolve_model(model_id: &str) -> Option<&'static ProviderModelSpec> {
    PROVIDER_MODELS.iter().find(|spec| spec.id == model_id)
}

pub fn resolve_request_target(model_id: &str) -> Option<ProviderRequestTarget> {
    resolve_model(model_id)
        .map(ProviderRequestTarget::Ask)
        .or_else(|| (model_id == RESEARCH_PRESET_ID).then_some(ProviderRequestTarget::Research))
}

pub fn model_object(
    spec: &ProviderModelSpec,
    report: Option<&CapabilityReport>,
    capability_source: &str,
) -> Value {
    let snapshot = model_snapshot_status(spec, report);
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
        "tablinq_preset_kind": "model_selection",
        "tablinq_ui_target": ui_target(spec),
        "tablinq_default_mode": "normal",
        "tablinq_mode": "normal",
        "tablinq_model_class": model_class_name(spec.model_class),
        "tablinq_effort": spec.effort.map(effort_name),
        "tablinq_execution_kind": "ask",
        "tablinq_supported_modes": ["normal", "search"],
        "tablinq_search_mode": true,
        "tablinq_search_mode_status": search_mode_status(report).availability,
        "tablinq_search_mode_kind": "composer_chip",
        "tablinq_search_mode_note": "Search is a ChatGPT composer chip/mode selected with tablinq_mode=\"search\", not a model id.",
        "tablinq_streaming": true,
        "tablinq_streaming_status": "final_only",
        "tablinq_live_streaming": false,
        "tablinq_files": files.availability == "available",
        "tablinq_files_status": files.availability,
        "tablinq_artifacts": artifacts.availability == "available",
        "tablinq_artifacts_status": artifacts.availability,
        "tablinq_snapshot_status": snapshot.snapshot_status,
        "tablinq_snapshot_note": snapshot.notes,
        "tablinq_capability_source": capability_source,
        "tablinq_aliases": observed_aliases(spec, report),
    })
}

pub fn model_snapshot_status(
    spec: &ProviderModelSpec,
    report: Option<&CapabilityReport>,
) -> CapabilitySnapshotStatus {
    let Some(report) = report else {
        return CapabilitySnapshotStatus {
            snapshot_status: "unknown",
            notes: vec![unknown_capability_note()],
        };
    };

    let available = match spec.id {
        "tablinq-instant" => composer_submission_ready(report),
        "tablinq-thinking-standard" | "tablinq-thinking-extended" => {
            composer_submission_ready(report)
                && thinking_effort_available(report, spec.effort.unwrap_or(Effort::None))
        }
        _ => false,
    };
    if available {
        CapabilitySnapshotStatus {
            snapshot_status: "observed",
            notes: Vec::new(),
        }
    } else {
        let note = match spec.id {
            "tablinq-instant" => {
                "the required instant submission state was not observed in the current attached browser-backed ChatGPT snapshot"
            }
            "tablinq-thinking-standard" | "tablinq-thinking-extended" => {
                if !composer_submission_ready(report) {
                    "the required thinking submission state was not observed in the current attached browser-backed ChatGPT snapshot"
                } else {
                    "the required thinking effort control was not observed in the current attached browser-backed ChatGPT snapshot"
                }
            }
            _ => {
                "the required model selection state was not observed in the current attached browser-backed ChatGPT snapshot"
            }
        };
        CapabilitySnapshotStatus {
            snapshot_status: "not_observed",
            notes: vec![note.to_owned()],
        }
    }
}

pub(super) fn composer_submission_ready(report: &CapabilityReport) -> bool {
    report.matrix.composer_ready && report.matrix.submit_available
}

pub(super) fn search_capability_available(report: &CapabilityReport) -> bool {
    let _ = report;
    true
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
    _unavailable_warning: &str,
) -> CapabilityAvailability {
    match report {
        Some(report) if predicate(report) => CapabilityAvailability {
            availability: "available",
        },
        Some(_) => CapabilityAvailability {
            availability: "unavailable",
        },
        None => CapabilityAvailability {
            availability: "unknown",
        },
    }
}

fn search_mode_status(report: Option<&CapabilityReport>) -> CapabilityAvailability {
    match report {
        Some(_) => CapabilityAvailability {
            availability: "available",
        },
        None => CapabilityAvailability {
            availability: "unknown",
        },
    }
}

fn unknown_capability_note() -> String {
    "the current browser-backed ChatGPT UI snapshot is unknown until the attached tab reports probes"
        .to_owned()
}

fn ui_target(spec: &ProviderModelSpec) -> Value {
    json!({
        "default_mode": "normal",
        "model_class": model_class_name(spec.model_class),
        "effort": spec.effort.map(effort_name),
        "supported_modes": ["normal", "search"],
        "search_mode": {
            "kind": "composer_chip",
            "request_field": "tablinq_mode",
            "request_value": "search",
        },
        "requires_research_entrypoint": false,
    })
}

fn observed_aliases(spec: &ProviderModelSpec, report: Option<&CapabilityReport>) -> Value {
    let Some(report) = report else {
        return json!({
            "modes": Vec::<String>::new(),
            "model_classes": Vec::<String>::new(),
            "efforts": Vec::<String>::new(),
        });
    };

    let model_class_key = match spec.model_class {
        ModelClass::Instant => "instant",
        ModelClass::Thinking => "thinking",
    };
    let effort_key = match spec.effort {
        Some(Effort::Standard) => Some("standard"),
        Some(Effort::Extended) => Some("extended"),
        None => None,
        Some(Effort::None) => None,
    };

    json!({
        "modes": report.aliases.modes.get("normal").cloned().unwrap_or_default(),
        "search_modes": report.aliases.modes.get("search").cloned().unwrap_or_default(),
        "model_classes": report.aliases.model_classes.get(model_class_key).cloned()
            .unwrap_or_default(),
        "efforts": effort_key
            .and_then(|key| report.aliases.efforts.get(key).cloned())
            .unwrap_or_default(),
    })
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
    use shared_protocol::{CapabilityMatrix, CapabilityObservedUi, RuntimeAliases};

    fn report_with_aliases(matrix: CapabilityMatrix, aliases: RuntimeAliases) -> CapabilityReport {
        CapabilityReport {
            reported_at: Utc::now(),
            selector_pack_id: "test-pack".to_owned(),
            selector_pack_version: "v1".to_owned(),
            matrix,
            aliases,
            observed_ui: CapabilityObservedUi::default(),
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
    fn base_model_availability_uses_live_capability_report_aliases() {
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
            model_snapshot_status(&PROVIDER_MODELS[0], Some(&report)).snapshot_status,
            "observed"
        );
        assert!(search_capability_available(&report));
        assert!(research_capability_available(&report));
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
            model_snapshot_status(&PROVIDER_MODELS[1], Some(&report)).snapshot_status,
            "observed"
        );
        assert_eq!(
            model_snapshot_status(&PROVIDER_MODELS[2], Some(&report)).snapshot_status,
            "not_observed"
        );
    }

    #[test]
    fn research_capability_requires_submit_ready_page_state() {
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

        assert!(!research_capability_available(&report));
    }

    #[test]
    fn search_capability_is_not_visibility_gated() {
        let report = report_with_aliases(
            CapabilityMatrix {
                composer_ready: true,
                submit_available: true,
                search_mode_visible: false,
                ..CapabilityMatrix::default()
            },
            RuntimeAliases::default(),
        );

        assert!(search_capability_available(&report));
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
        assert_eq!(model["tablinq_streaming_status"], "final_only");
        assert_eq!(model["tablinq_live_streaming"], false);
        assert_eq!(model["tablinq_files"], false);
        assert_eq!(model["tablinq_files_status"], "unavailable");
        assert_eq!(model["tablinq_artifacts"], false);
        assert_eq!(model["tablinq_artifacts_status"], "unavailable");
    }
}
