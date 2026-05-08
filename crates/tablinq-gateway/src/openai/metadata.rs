use serde::Serialize;
use serde_json::{Value, json};
use shared_protocol::CapabilityReport;

use super::models::{
    PROVIDER_MODELS, model_snapshot_status, research_capability_available,
    search_capability_available,
};
use super::request::RequestRole;

pub fn provider_capability_catalog(
    report: Option<&CapabilityReport>,
    capability_source: &str,
) -> Value {
    let preset_ids = PROVIDER_MODELS
        .iter()
        .map(|spec| spec.id)
        .collect::<Vec<_>>();
    let observed_preset_ids = PROVIDER_MODELS
        .iter()
        .filter(|spec| model_snapshot_status(spec, report).snapshot_status == "observed")
        .map(|spec| spec.id)
        .collect::<Vec<_>>();

    json!({
        "capability_source": capability_source,
        "preset_ids": preset_ids.clone(),
        "observed_preset_ids": observed_preset_ids,
        "capabilities": {
            "search": capability_status(
                report,
                "tablinq-search",
                search_capability_available,
                "the Search composer mode was not observed in the current attached browser-backed ChatGPT snapshot",
            ),
            "research": capability_status(
                report,
                "tablinq-research",
                research_capability_available,
                "the Deep Research entrypoint or ready research page state was not observed in the current attached browser-backed ChatGPT snapshot",
            ),
            "uploads": capability_status(
                report,
                "all_models",
                |value| value.matrix.upload_control_visible,
                "file upload controls were not observed in the current attached browser-backed ChatGPT snapshot",
            ),
            "artifacts": capability_status(
                report,
                "all_models",
                |value| value.matrix.downloadable_artifacts_observable,
                "downloadable artifacts were not observed in the current attached browser-backed ChatGPT snapshot",
            ),
            "streaming": json!({
                "gateway_support": "supported",
                "stable_preset_ids": preset_ids,
                "note": "streaming responses are supported by the gateway, but live execution still depends on an attached browser-backed ChatGPT tab",
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
    stable_preset_ids: impl Serialize,
    predicate: impl FnOnce(&CapabilityReport) -> bool,
    not_observed_note: &str,
) -> Value {
    match report {
        Some(report) if predicate(report) => json!({
            "snapshot_status": "observed",
            "stable_preset_ids": stable_preset_ids,
            "note": Value::Null,
        }),
        Some(_) => json!({
            "snapshot_status": "not_observed",
            "stable_preset_ids": stable_preset_ids,
            "note": not_observed_note,
        }),
        None => json!({
            "snapshot_status": "unknown",
            "stable_preset_ids": stable_preset_ids,
            "note": "the current browser-backed ChatGPT UI snapshot is unknown until the attached tab reports probes",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use shared_protocol::{CapabilityMatrix, CapabilityObservedUi, RuntimeAliases};
    use std::collections::BTreeMap;

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

    #[test]
    fn provider_catalog_keeps_missing_modes_not_observed_without_aliases() {
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
            provider["capabilities"]["search"]["snapshot_status"],
            "not_observed"
        );
        assert_eq!(
            provider["capabilities"]["research"]["snapshot_status"],
            "not_observed"
        );
    }

    #[test]
    fn provider_catalog_uses_attach_first_queue_wording() {
        let provider = provider_capability_catalog(None, "unknown");
        let text = provider.to_string();

        assert_eq!(provider["subagent_recommendation"]["policy"], "queue");
        assert!(
            provider["capabilities"]["search"]["note"]
                .as_str()
                .is_some_and(|value| value.contains("browser-backed"))
        );
        assert!(!text.contains("managed"));
    }
}
