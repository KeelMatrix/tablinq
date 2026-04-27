use crate::http::HttpResponse;
use crate::json::json_response;
use crate::openai::{RequestRole, openai_error};
use serde_json::{Value, json};

pub(crate) fn request_role_policy_response(
    request_id: &str,
    role: RequestRole,
) -> Option<HttpResponse> {
    match role {
        RequestRole::Primary | RequestRole::Branch => None,
        RequestRole::Subagent => {
            let mut body = openai_error(
                429,
                "overloaded_error",
                "subagents_disabled",
                &format!(
                    "{request_id}: subagent requests are disabled by default for the single-browser OpenClaude preset"
                ),
            );
            body["request_id"] = Value::String(request_id.to_owned());
            body["tablinq_refusal_reason"] = Value::String("subagents_disabled".to_owned());
            body["tablinq_guidance"] = json!(provider_guidance_for_code("subagents_disabled"));
            Some(json_response(429, body))
        }
    }
}

pub(crate) fn provider_guidance_for_code(code: &str) -> Vec<&'static str> {
    match code {
        "auth_required" => vec![
            "provider mode refuses requests when the managed browser is signed out; sign in in the managed profile and retry",
            "run gpt doctor or GET /ready to confirm the browser session is authenticated before retrying queued work",
        ],
        "capability_missing" => vec![
            "provider mode refuses requests when the selected model or mode is unavailable instead of guessing a fallback",
            "refresh GET /v1/capabilities after account, workspace, or UI changes before retrying the request",
        ],
        "browser_unavailable" => vec![
            "provider mode refuses requests until the managed browser can be opened exclusively",
            "close other managed-profile browser instances or reset the managed profile if the browser stays unavailable",
        ],
        "upstream_timeout" => vec![
            "provider mode refuses long-stalled requests instead of inventing a partial answer",
            "inspect the latest failure bundle and retry after the managed tab is responsive again",
        ],
        "ui_drift" => vec![
            "provider mode refuses when the live page no longer matches known controls",
            "inspect the linked failure bundle before retrying so selector or capability drift is actionable",
        ],
        "research_job_failed" => vec![
            "provider mode refuses failed research runs instead of returning incomplete research output",
        ],
        "subagents_disabled" => vec![
            "provider mode keeps subagents disabled by default to protect the single warm browser session",
        ],
        _ => Vec::new(),
    }
}

pub(crate) fn refusal_reason_for_code(code: &str) -> Option<&str> {
    match code {
        "auth_required"
        | "capability_missing"
        | "browser_unavailable"
        | "upstream_timeout"
        | "ui_drift"
        | "research_job_failed"
        | "subagents_disabled" => Some(code),
        _ => None,
    }
}
