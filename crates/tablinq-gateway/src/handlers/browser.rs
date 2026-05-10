use crate::browser::recommended_cli_browser_setup;
use crate::core_status::effective_gateway_admin_snapshot;
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
    config: &AppConfig,
    core: &CoreService,
    request_id: &str,
) -> Result<HttpResponse> {
    let snapshot = effective_gateway_admin_snapshot(config, core)?;
    Ok(browser_snapshot_response(snapshot.browser, request_id))
}

pub(crate) fn handle_browser_connection_info_request(
    config: &AppConfig,
    core: &CoreService,
    request_id: &str,
) -> Result<HttpResponse> {
    let snapshot = effective_gateway_admin_snapshot(config, core)?;
    Ok(browser_connection_info_response(
        config,
        snapshot.browser,
        request_id,
    ))
}

fn browser_connection_info_response(
    config: &AppConfig,
    browser: shared_protocol::BrowserStatusSnapshot,
    request_id: &str,
) -> HttpResponse {
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

#[cfg(test)]
mod tests {
    use super::*;
    use shared_protocol::{AuthSource, BrowserConnectionMode, BrowserOwner};
    use tempfile::tempdir;

    #[test]
    fn profile_and_connection_info_responses_expose_same_attached_browser_state() {
        let dir = tempdir().unwrap();
        let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        let browser = attached_browser_snapshot();

        let profile = browser_snapshot_response(browser.clone(), "req-profile");
        let connection = browser_connection_info_response(&config, browser, "req-connection");
        let profile_json: serde_json::Value = serde_json::from_slice(&profile.body).unwrap();
        let connection_json: serde_json::Value = serde_json::from_slice(&connection.body).unwrap();

        assert_attached_browser_fields(&profile_json["data"]);
        assert_attached_browser_fields(&connection_json["browser"]);
    }

    fn assert_attached_browser_fields(browser: &serde_json::Value) {
        assert_eq!(browser["authenticated"], true);
        assert_eq!(browser["cdp_attached"], true);
        assert_eq!(browser["extension_connected"], true);
        assert_eq!(browser["page_ready"], true);
        assert_eq!(browser["running"], true);
        assert_eq!(browser["target_tab_found"], true);
        assert_ne!(browser["auth_source"], "unknown");
    }

    fn attached_browser_snapshot() -> shared_protocol::BrowserStatusSnapshot {
        shared_protocol::BrowserStatusSnapshot {
            running: true,
            browser_kind: "chrome".to_owned(),
            profile_dir: "profile".to_owned(),
            attach_profile_dir: None,
            attach_profile_note: None,
            downloads_dir: "downloads".to_owned(),
            target_url: "https://chatgpt.com/".to_owned(),
            current_url: Some("https://chatgpt.com/c/test".to_owned()),
            page_title: Some("ChatGPT".to_owned()),
            page_ready: true,
            authenticated: true,
            reload_count: 0,
            screenshot_path: None,
            browser_path: None,
            extension_connected: true,
            capability_report: None,
            attach_mode: BrowserConnectionMode::AttachFirst,
            browser_owner: BrowserOwner::External,
            cdp_attached: true,
            target_tab_found: true,
            target_tab_id: Some("tab-1".to_owned()),
            target_workspace_key: None,
            debug_endpoint: Some("http://127.0.0.1:9222".to_owned()),
            auth_source: AuthSource::Page,
            extension_required: true,
            last_capability_report_at: None,
            action: None,
            browser_process_closed: None,
            reason: None,
        }
    }
}
