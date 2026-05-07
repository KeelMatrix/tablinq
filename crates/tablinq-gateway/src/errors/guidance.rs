pub(crate) fn provider_guidance_for_code(code: &str) -> Vec<&'static str> {
    match code {
        "auth_required" => vec![
            "browser-backed requests need an attached, signed-in ChatGPT tab before work can run",
            "check GET /ready or GET /v1/browser/connection-info, then run `gpt browser start-debug` and `gpt browser attach` if needed",
        ],
        "capability_missing" => vec![
            "provider mode refuses requests when the selected model or mode is unavailable instead of guessing a fallback",
            "refresh GET /v1/capabilities after account, workspace, or UI changes before retrying the request",
        ],
        "browser_unavailable" => vec![
            "browser-backed requests cannot run until the attach-first browser connection is available",
            "make sure the extension is connected, an eligible ChatGPT tab is open, and the local debug browser is attached",
        ],
        "upstream_timeout" => vec![
            "provider mode refuses long-stalled requests instead of inventing a partial answer",
            "inspect the latest failure bundle and retry after the attached ChatGPT tab is responsive again",
        ],
        "ui_drift" => vec![
            "provider mode refuses when the live page no longer matches known controls",
            "inspect the linked failure bundle before retrying so selector or capability drift is actionable",
        ],
        "research_job_failed" => vec![
            "provider mode refuses failed research runs instead of returning incomplete research output",
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
        | "research_job_failed" => Some(code),
        _ => None,
    }
}
