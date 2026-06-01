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
    let model_ids = PROVIDER_MODELS
        .iter()
        .map(|spec| spec.id)
        .collect::<Vec<_>>();
    let observed_model_ids = PROVIDER_MODELS
        .iter()
        .filter(|spec| model_snapshot_status(spec, report).snapshot_status == "observed")
        .map(|spec| spec.id)
        .collect::<Vec<_>>();

    json!({
        "capability_source": capability_source,
        "model_ids": model_ids.clone(),
        "observed_model_ids": observed_model_ids,
        "capabilities": {
            "search": search_capability_status(
                report,
                model_ids.clone(),
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
                "gateway_support": "final_only",
                "stable_model_ids": model_ids,
                "live_token_streaming": false,
                "note": "stream:true is accepted for OpenAI-compatible SSE framing, but Tablinq waits for the finalized browser-backed response and emits final-only chunks because ChatGPT drafts may be rewritten before finalization",
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

fn search_capability_status(
    report: Option<&CapabilityReport>,
    stable_model_ids: Vec<&str>,
) -> Value {
    match report {
        Some(report) if search_capability_available(report) => json!({
            "snapshot_status": "observed",
            "stable_model_ids": stable_model_ids,
            "kind": "composer_chip",
            "request_field": "tablinq_mode",
            "request_value": "search",
            "note": "Search is a ChatGPT composer chip/mode selected with tablinq_mode=\"search\" alongside a base model; it is not a model id.",
        }),
        Some(_) => json!({
            "snapshot_status": "not_observed",
            "stable_model_ids": stable_model_ids,
            "kind": "composer_chip",
            "request_field": "tablinq_mode",
            "request_value": "search",
            "note": "Search is a ChatGPT composer chip/mode, but it was not observed in the current attached browser-backed ChatGPT snapshot.",
        }),
        None => json!({
            "snapshot_status": "unknown",
            "stable_model_ids": stable_model_ids,
            "kind": "composer_chip",
            "request_field": "tablinq_mode",
            "request_value": "search",
            "note": "Search is a ChatGPT composer chip/mode selected with tablinq_mode=\"search\" alongside a base model; the current browser-backed ChatGPT UI snapshot is unknown.",
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
    fn provider_catalog_reports_search_as_request_mode_metadata() {
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
            "observed"
        );
        assert_eq!(provider["capabilities"]["search"]["kind"], "composer_chip");
        assert_eq!(
            provider["capabilities"]["search"]["request_field"],
            "tablinq_mode"
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
