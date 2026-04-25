use crate::capabilities::{CapabilityCatalog, CapabilityCatalogCache};
use crate::http::HttpResponse;
use crate::json::json_response;
use crate::openai::{PROVIDER_MODELS, model_object, provider_capability_catalog};
use crate::process_control::GatewayStatusSnapshot;
use crate::routes::REQUEST_SUMMARY_LIMIT;
use anyhow::Result;
use gptd::CoreService;
use serde_json::json;

pub(crate) fn handle_models_request(
    core: &CoreService,
    capability_cache: &CapabilityCatalogCache,
    snapshot: &GatewayStatusSnapshot,
    request_id: &str,
) -> HttpResponse {
    let catalog = capability_cache
        .resolve(core)
        .unwrap_or_else(|_| CapabilityCatalog {
            source: "unknown",
            cache_key: "unavailable".to_owned(),
            profile_dir: core
                .config()
                .paths
                .browser_profile_dir
                .display()
                .to_string(),
            workspace_key: core.config().browser_target_url().to_owned(),
            report: None,
        });
    let models = PROVIDER_MODELS
        .iter()
        .map(|spec| model_object(spec, catalog.report.as_ref(), catalog.source))
        .collect::<Vec<_>>();
    json_response(
        200,
        json!({
            "object": "list",
            "data": models,
            "request_id": request_id,
            "tablinq_capability_source": catalog.source,
            "tablinq_model_ids": PROVIDER_MODELS.iter().map(|spec| spec.id).collect::<Vec<_>>(),
            "gateway": {
                "host": snapshot.host,
                "port": snapshot.port,
            }
        }),
    )
}

pub(crate) fn handle_capabilities_request(
    core: &CoreService,
    capability_cache: &CapabilityCatalogCache,
    request_id: &str,
) -> Result<HttpResponse> {
    let catalog = capability_cache.resolve(core)?;
    let aliases = catalog
        .report
        .as_ref()
        .map(|report| report.aliases.clone())
        .unwrap_or_default();
    let models = PROVIDER_MODELS
        .iter()
        .map(|spec| model_object(spec, catalog.report.as_ref(), catalog.source))
        .collect::<Vec<_>>();

    Ok(json_response(
        200,
        json!({
            "object": "tablinq.capability_catalog",
            "request_id": request_id,
            "source": catalog.source,
            "cache_key": catalog.cache_key,
            "profile_dir": catalog.profile_dir,
            "workspace_key": catalog.workspace_key,
            "reported_at": catalog.report.as_ref().map(|report| report.reported_at),
            "selector_pack_id": catalog.report.as_ref().map(|report| report.selector_pack_id.clone()),
            "selector_pack_version": catalog.report.as_ref().map(|report| report.selector_pack_version.clone()),
            "aliases": aliases,
            "models": models,
            "provider": provider_capability_catalog(catalog.report.as_ref(), catalog.source),
        }),
    ))
}

pub(crate) fn handle_provider_request(
    core: &CoreService,
    capability_cache: &CapabilityCatalogCache,
    snapshot: &GatewayStatusSnapshot,
    request_id: &str,
) -> HttpResponse {
    let catalog = capability_cache
        .resolve(core)
        .unwrap_or_else(|_| CapabilityCatalog {
            source: "unknown",
            cache_key: "unavailable".to_owned(),
            profile_dir: core
                .config()
                .paths
                .browser_profile_dir
                .display()
                .to_string(),
            workspace_key: core.config().browser_target_url().to_owned(),
            report: None,
        });
    json_response(
        200,
        json!({
            "object": "tablinq.provider",
            "request_id": request_id,
            "provider": "tablinq",
            "openai_compatible": true,
            "base_url": format!("http://{}:{}/v1", snapshot.host, snapshot.port),
            "capability_source": catalog.source,
            "model_ids": PROVIDER_MODELS.iter().map(|spec| spec.id).collect::<Vec<_>>(),
            "provider_capabilities": provider_capability_catalog(catalog.report.as_ref(), catalog.source),
            "refusal_conditions": [
                {
                    "code": "auth_required",
                    "behavior": "refuse",
                    "when": "the managed browser session is signed out or has expired",
                },
                {
                    "code": "capability_missing",
                    "behavior": "refuse",
                    "when": "the selected live account or workspace no longer exposes the requested model or mode",
                },
                {
                    "code": "ui_drift",
                    "behavior": "refuse",
                    "when": "the live page no longer matches the known automation controls",
                },
                {
                    "code": "upstream_timeout",
                    "behavior": "refuse",
                    "when": "the managed page stalls instead of producing a stable streamed response",
                }
            ],
            "request_annotations": {
                "field": "tablinq_request_role",
                "parent_request_id_field": "tablinq_parent_request_id",
                "allowed_values": ["primary", "subagent", "branch"],
                "default": "primary",
                "policy": {
                    "primary": "execute",
                    "branch": "serialize",
                    "subagent": "reject",
                }
            },
            "presets": {
                "openclaude": {
                    "recommended_models": [
                        "tablinq-instant",
                        "tablinq-thinking-standard",
                        "tablinq-search",
                    ],
                    "streaming": true,
                    "max_parallel_requests": 1,
                    "subagents": "disabled_by_default",
                    "subagent_recommendation": "disable",
                },
                "paperclip": {
                    "transport": "openai_compatible_http",
                    "recommended_models": [
                        "tablinq-instant",
                        "tablinq-search",
                        "tablinq-research",
                    ],
                    "streaming": true,
                    "max_parallel_requests": 1,
                    "artifacts": true,
                    "files": true,
                    "usage_accounting": "unknown_or_estimated",
                    "subagent_recommendation": "disable",
                    "budget_guidance": {
                        "rate_limit_hint": "keep provider concurrency at 1",
                        "retry_backoff_hint": "respect queue_timeout and retry_after_ms on 429 responses",
                    }
                }
            }
        }),
    )
}

pub(crate) fn handle_usage_request(core: &CoreService, request_id: &str) -> Result<HttpResponse> {
    let history = core.list_request_summaries(REQUEST_SUMMARY_LIMIT)?;
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.request_summary_list",
            "request_id": request_id,
            "data": history.requests,
            "limit": REQUEST_SUMMARY_LIMIT,
            "warnings": ["token usage is unavailable for the browser-backed provider"],
        }),
    ))
}

pub(crate) fn handle_sessions_request(
    core: &CoreService,
    request_id: &str,
) -> Result<HttpResponse> {
    let sessions = core.list_sessions()?;
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.session_list",
            "request_id": request_id,
            "data": sessions.sessions,
        }),
    ))
}
