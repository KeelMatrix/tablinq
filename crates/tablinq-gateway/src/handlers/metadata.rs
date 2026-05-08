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
            "tablinq_probe_reported_at": catalog.report.as_ref().map(|report| report.reported_at),
            "tablinq_preset_ids": PROVIDER_MODELS.iter().map(|spec| spec.id).collect::<Vec<_>>(),
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
            "observed_ui": catalog.report.as_ref().map(|report| report.observed_ui.clone()),
            "aliases": aliases,
            "presets": models,
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
            "base_url": format!("http://{}:{}/v1", snapshot.host, snapshot.port),
            "reported_at": catalog.report.as_ref().map(|report| report.reported_at),
            "capability_source": catalog.source,
            "preset_ids": PROVIDER_MODELS.iter().map(|spec| spec.id).collect::<Vec<_>>(),
            "openai_compatibility": {
                "mode": "subset",
                "supported_endpoints": [
                    "/v1/chat/completions",
                    "/v1/models",
                    "/v1/provider",
                    "/v1/capabilities",
                    "/v1/files",
                    "/v1/artifacts",
                    "/v1/requests",
                ],
                "chat_completions": {
                    "input_handling": [
                        "chat_completions input is reduced to user-authored text content before browser submission",
                        "browser submission preserves message text exactly and does not add transcript wrappers",
                        "extra conversation context is not supported in browser submission",
                        "unsupported message context is rejected before queueing",
                        "stream=true is rejected before queueing because browser-backed ChatGPT drafts may be rewritten before finalization",
                    ],
                    "tolerated_parameters": [
                        "common generation tuning parameters",
                        "client metadata parameters",
                        "stream shaping parameters when stream remains false",
                    ],
                    "unsupported_parameters": [
                        "live streaming responses",
                        "function-call payloads",
                        "structured output formats",
                        "multiple-choice fanout",
                    ],
                    "constraints": [
                        "n must be 1",
                        "stream must be false or omitted",
                        "only user-authored text content is forwarded to the browser",
                    ],
                },
                "files": {
                    "endpoint": "/v1/files",
                    "upload_shape": "raw_request_body",
                    "notes": [
                        "upload bytes directly to stage a file id",
                        "re-use staged ids via tablinq_file_ids or input_file content parts",
                    ],
                },
            },
            "provider_capabilities": provider_capability_catalog(catalog.report.as_ref(), catalog.source),
            "execution": {
                "connection_model": "attach_first_browser_backed",
                "readiness_endpoints": ["/ready", "/v1/browser/connection-info"],
                "operator_commands": ["gpt browser start-debug", "gpt browser attach"],
                "requirements": [
                    "a connected browser extension",
                    "a signed-in ChatGPT tab",
                ],
            },
            "refusal_conditions": [
                {
                    "code": "auth_required",
                    "behavior": "refuse",
                    "when": "the attached browser-backed ChatGPT tab is signed out, detached, or no longer authenticated",
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
                    "when": "the attached ChatGPT tab stalls instead of producing a finalized response",
                }
            ],
            "request_queueing": {
                "default_behavior": "execute_or_queue",
                "parallel_work": "queue",
                "dependent_work": "queue",
                "notes": [
                    "valid API requests are accepted into the durable queue",
                    "single-flight browser-backed execution keeps effective concurrency at 1",
                ],
            },
            "presets": {
                "openclaude": {
                    "recommended_presets": [
                        "tablinq-instant",
                        "tablinq-thinking-standard",
                        "tablinq-search",
                    ],
                    "streaming": false,
                    "streaming_note": "set stream to false or omit it; browser-backed ChatGPT drafts may be rewritten before finalization",
                    "max_parallel_requests": 1,
                    "subagents": "queued",
                    "subagent_recommendation": "queue",
                },
                "paperclip": {
                    "transport": "openai_compatible_http",
                    "recommended_presets": [
                        "tablinq-instant",
                        "tablinq-search",
                        "tablinq-research",
                    ],
                    "streaming": false,
                    "streaming_note": "set stream to false or omit it; browser-backed ChatGPT drafts may be rewritten before finalization",
                    "max_parallel_requests": 1,
                    "artifacts": true,
                    "files": true,
                    "usage_accounting": "unknown",
                    "subagent_recommendation": "queue",
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
