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

pub(super) fn composer_submission_ready(report: &CapabilityReport) -> bool {
    report.matrix.composer_ready && report.matrix.submit_available
}

pub(super) fn search_capability_available(report: &CapabilityReport) -> bool {
    composer_submission_ready(report)
        && (report.matrix.search_mode_visible || has_mode_alias(report, "search"))
}

pub(super) fn research_capability_available(report: &CapabilityReport) -> bool {
    report.matrix.research_mode_visible || has_mode_alias(report, "research")
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
}
