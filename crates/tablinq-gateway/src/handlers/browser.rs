use crate::browser::recommended_cli_browser_setup;
use crate::http::HttpResponse;
use crate::json::{api_error_response, json_response};
use anyhow::Result;
use gptd::{AppConfig, CoreService};
use serde::Deserialize;
use serde_json::json;
use shared_protocol::BrowserOpenRequest;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BrowserAttachRequest {
    #[serde(default)]
    target_url: Option<String>,
    #[serde(default)]
    browser_path: Option<String>,
    #[serde(default)]
    headless: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct BrowserDetachRequest {}

pub(crate) fn handle_browser_attach_request(
    core: &CoreService,
    body: &[u8],
    request_id: &str,
) -> Result<HttpResponse> {
    let request = if body.is_empty() {
        BrowserOpenRequest {
            target_url: None,
            browser_path: None,
            headless: None,
        }
    } else {
        let request = match serde_json::from_slice::<BrowserAttachRequest>(body) {
            Ok(request) => request,
            Err(err) => {
                return Ok(api_error_response(
                    400,
                    request_id,
                    "invalid_request_error",
                    "invalid_request_json",
                    format!("{err:#}"),
                ));
            }
        };
        BrowserOpenRequest {
            target_url: request.target_url,
            browser_path: request.browser_path,
            headless: request.headless,
        }
    };
    let snapshot = core.browser_open(request)?;
    Ok(browser_snapshot_response(snapshot, request_id))
}

pub(crate) fn handle_browser_detach_request(
    core: &CoreService,
    body: &[u8],
    request_id: &str,
) -> Result<HttpResponse> {
    if !body.is_empty() {
        match serde_json::from_slice::<BrowserDetachRequest>(body) {
            Ok(_) => {}
            Err(err) => {
                return Ok(api_error_response(
                    400,
                    request_id,
                    "invalid_request_error",
                    "invalid_request_json",
                    format!("{err:#}"),
                ));
            }
        }
    }
    let snapshot = core.browser_close()?;
    Ok(browser_snapshot_response(snapshot, request_id))
}

pub(crate) fn handle_browser_profile_info_request(
    core: &CoreService,
    request_id: &str,
) -> Result<HttpResponse> {
    let snapshot = core.browser_profile_info()?;
    Ok(browser_snapshot_response(snapshot, request_id))
}

pub(crate) fn handle_browser_connection_info_request(
    config: &AppConfig,
    core: &CoreService,
    request_id: &str,
) -> HttpResponse {
    let browser = core.browser_profile_info().ok();
    json_response(
        200,
        json!({
            "object": "tablinq.browser_connection_info",
            "request_id": request_id,
            "debug_endpoint": config.browser_debug_endpoint(),
            "target_url": config.browser_target_url(),
            "browser_channel": config.browser_channel(),
            "browser_path": config.browser_path(),
            "extension_required": config.browser_extension_required(),
            "browser": browser,
            "recommended_cli_setup": recommended_cli_browser_setup(config),
        }),
    )
}

fn browser_snapshot_response(
    snapshot: shared_protocol::BrowserStatusSnapshot,
    request_id: &str,
) -> HttpResponse {
    json_response(
        200,
        json!({
            "object": "tablinq.browser",
            "request_id": request_id,
            "data": snapshot,
        }),
    )
}
