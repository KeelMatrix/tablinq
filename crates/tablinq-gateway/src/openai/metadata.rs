use serde::Serialize;
use serde_json::{Value, json};
use shared_protocol::CapabilityReport;

use super::models::{
    PROVIDER_MODELS, model_availability, research_capability_available, search_capability_available,
};
use super::request::RequestRole;

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
                "search mode is unavailable in the attached browser-backed ChatGPT tab",
            ),
            "research": capability_status(
                report,
                "tablinq-research",
                research_capability_available,
                "research mode is unavailable in the attached browser-backed ChatGPT tab",
            ),
            "uploads": capability_status(
                report,
                "all_models",
                |value| value.matrix.upload_control_visible,
                "file upload controls are unavailable in the attached browser-backed ChatGPT tab",
            ),
            "artifacts": capability_status(
                report,
                "all_models",
                |value| value.matrix.downloadable_artifacts_observable,
                "downloadable artifacts are unavailable in the attached browser-backed ChatGPT tab",
            ),
            "streaming": json!({
                "availability": "available",
                "stable_model_ids": model_ids,
                "warning": "streaming responses are supported by the gateway, but live execution still depends on an attached browser-backed ChatGPT tab",
            }),
        },
        "max_safe_parallelism": {
            "requests": 1,
            "branches": 1,
            "subagents": 0,
        },
        "subagent_recommendation": {
            "policy": "queue",
            "reason": "single-flight execution against one attached browser-backed ChatGPT tab keeps effective concurrency at 1",
        },
    })
}

pub fn request_role_name(role: RequestRole) -> &'static str {
    match role {
        RequestRole::Primary => "primary",
        RequestRole::Subagent => "subagent",
        RequestRole::Branch => "branch",
    }
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
            "warning": "live capability availability is unknown until the attached browser-backed ChatGPT tab reports probes",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use shared_protocol::{CapabilityMatrix, RuntimeAliases};
    use std::collections::BTreeMap;

    fn report_with_aliases(matrix: CapabilityMatrix, aliases: RuntimeAliases) -> CapabilityReport {
        CapabilityReport {
            reported_at: Utc::now(),
            selector_pack_id: "test-pack".to_owned(),
            selector_pack_version: "v1".to_owned(),
            matrix,
            aliases,
        }
    }

    #[test]
    fn provider_catalog_keeps_missing_modes_unavailable_without_aliases() {
        let report = report_with_aliases(
            CapabilityMatrix {
                composer_ready: true,
                submit_available: true,
                ..CapabilityMatrix::default()
            },
            RuntimeAliases {
                modes: BTreeMap::from([("normal".to_owned(), vec!["chat".to_owned()])]),
                ..RuntimeAliases::default()
            },
        );

        let provider = provider_capability_catalog(Some(&report), "live");
        assert_eq!(
            provider["capabilities"]["search"]["availability"],
            "unavailable"
        );
        assert_eq!(
            provider["capabilities"]["research"]["availability"],
            "unavailable"
        );
    }

    #[test]
    fn provider_catalog_uses_attach_first_queue_wording() {
        let provider = provider_capability_catalog(None, "unknown");
        let text = provider.to_string();

        assert_eq!(provider["subagent_recommendation"]["policy"], "queue");
        assert!(
            provider["capabilities"]["search"]["warning"]
                .as_str()
                .is_some_and(|value| value.contains("browser-backed"))
        );
        assert!(!text.contains("managed"));
    }
}
