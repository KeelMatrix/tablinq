use super::RequestLifecycleUpdate;
use crate::config::AppConfig;
use shared_protocol::{
    AskCommand, AuthSource, BrowserConnectionMode, BrowserOwner, BrowserStatusSnapshot,
    CHROME_ATTACH_PROFILE_NOTE, Effort,
};

pub(super) fn resolve_ask_timeout_ms(config: &AppConfig, command: &AskCommand) -> u64 {
    match command.effort {
        Some(Effort::Extended) => config.extended_thinking_timeout_ms(),
        _ => config.ask_timeout_ms(),
    }
}

pub(super) fn is_recoverable_adapter_error(message: &str) -> bool {
    message.contains("Execution context was destroyed")
        || message.contains("Cannot find context")
        || message.contains("Frame has been detached")
        || message.contains("navigation")
        || message.contains("target page is not open")
        || message.contains("Target page, context or browser has been closed")
        || message.contains("browser adapter closed without")
}

pub(super) fn derive_recovery_conversation_id(
    command_conversation_id: Option<&str>,
    latest_update: Option<&RequestLifecycleUpdate>,
    current_url: Option<&str>,
) -> Option<String> {
    command_conversation_id
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            latest_update
                .and_then(|update| update.conversation_id.as_deref())
                .filter(|value| !value.trim().is_empty())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            latest_update
                .and_then(|update| update.current_url.as_deref())
                .and_then(conversation_id_from_url)
        })
        .or_else(|| current_url.and_then(conversation_id_from_url))
}

pub(super) fn conversation_id_from_url(raw_url: &str) -> Option<String> {
    let marker = "/c/";
    let start = raw_url.find(marker)? + marker.len();
    let rest = &raw_url[start..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let value = rest[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

pub(super) fn should_reset_after_failure(message: &str) -> bool {
    message.contains("cancelled")
        || message.contains("target page is not open")
        || message.contains("Target page, context or browser has been closed")
        || message.contains("browser adapter closed without")
}

pub(super) fn idle_snapshot(
    config: &AppConfig,
    target_url: Option<String>,
    browser_path: Option<String>,
) -> BrowserStatusSnapshot {
    normalize_snapshot(
        config,
        BrowserStatusSnapshot {
            running: false,
            browser_kind: configured_browser_kind(config, browser_path.as_deref()),
            profile_dir: config.paths.browser_profile_dir.display().to_string(),
            attach_profile_dir: None,
            attach_profile_note: None,
            downloads_dir: config.paths.downloads_dir.display().to_string(),
            target_url: target_url.unwrap_or_else(|| config.browser_target_url().to_owned()),
            current_url: None,
            page_title: None,
            page_ready: false,
            authenticated: false,
            reload_count: 0,
            screenshot_path: None,
            browser_path,
            extension_connected: false,
            capability_report: None,
            attach_mode: config.browser_connection_mode(),
            browser_owner: BrowserOwner::Managed,
            cdp_attached: false,
            target_tab_found: false,
            target_tab_id: None,
            target_workspace_key: None,
            debug_endpoint: Some(config.browser_debug_endpoint()),
            auth_source: AuthSource::Unknown,
            extension_required: config.browser_extension_required(),
            last_capability_report_at: None,
            action: None,
            browser_process_closed: None,
            reason: None,
        },
    )
}

pub(super) fn normalize_snapshot(
    config: &AppConfig,
    mut snapshot: BrowserStatusSnapshot,
) -> BrowserStatusSnapshot {
    snapshot.attach_mode = config.browser_connection_mode();
    if snapshot.debug_endpoint.is_none() {
        snapshot.debug_endpoint = Some(config.browser_debug_endpoint());
    }
    if snapshot.running {
        snapshot.target_tab_found = snapshot.target_tab_found || snapshot.page_ready;
    } else {
        snapshot.target_tab_found = false;
        snapshot.auth_source = AuthSource::Unknown;
        snapshot.target_workspace_key = None;
        snapshot.last_capability_report_at = None;
    }
    snapshot.extension_required = config.browser_extension_required();
    if matches!(
        snapshot.attach_mode,
        BrowserConnectionMode::AttachFirst | BrowserConnectionMode::ManagedFallback
    ) && !snapshot.running
    {
        snapshot.browser_owner = BrowserOwner::External;
    }
    if uses_chrome_attach_profile(config, snapshot.browser_path.as_deref()) {
        snapshot.attach_profile_dir =
            Some(config.paths.chrome_attach_profile_dir.display().to_string());
        snapshot.attach_profile_note = Some(CHROME_ATTACH_PROFILE_NOTE.to_owned());
    } else {
        snapshot.attach_profile_dir = None;
        snapshot.attach_profile_note = None;
    }
    snapshot
}

fn configured_browser_kind(config: &AppConfig, browser_path: Option<&str>) -> String {
    if let Some(browser_path) = browser_path {
        if executable_looks_like_edge(browser_path) {
            return "msedge".to_owned();
        }
        if executable_looks_like_chrome(browser_path) {
            return "chrome".to_owned();
        }
    }
    if let Some(browser_path) = config.browser_path() {
        if executable_looks_like_edge(browser_path) {
            return "msedge".to_owned();
        }
        if executable_looks_like_chrome(browser_path) {
            return "chrome".to_owned();
        }
    }
    match config.browser_channel().to_ascii_lowercase().as_str() {
        "chrome" => "chrome".to_owned(),
        "msedge" | "edge" => "msedge".to_owned(),
        value if !value.is_empty() => value.to_owned(),
        _ => {
            if cfg!(windows) {
                "msedge".to_owned()
            } else {
                "chromium".to_owned()
            }
        }
    }
}

fn uses_chrome_attach_profile(config: &AppConfig, browser_path: Option<&str>) -> bool {
    if !matches!(
        config.browser_connection_mode(),
        BrowserConnectionMode::AttachFirst | BrowserConnectionMode::ManagedFallback
    ) {
        return false;
    }
    if let Some(browser_path) = browser_path {
        if executable_looks_like_edge(browser_path) {
            return false;
        }
        if executable_looks_like_chrome(browser_path) {
            return true;
        }
    }
    if let Some(browser_path) = config.browser_path() {
        if executable_looks_like_edge(browser_path) {
            return false;
        }
        if executable_looks_like_chrome(browser_path) {
            return true;
        }
    }
    config.browser_channel().eq_ignore_ascii_case("chrome")
}

fn executable_looks_like_edge(executable: &str) -> bool {
    let lower = executable.to_ascii_lowercase();
    lower.contains("msedge") || lower.contains("microsoft edge")
}

fn executable_looks_like_chrome(executable: &str) -> bool {
    let lower = executable.to_ascii_lowercase();
    lower.contains("google chrome") || lower.contains("chrome.exe") || lower.contains("chrome")
}

#[cfg(test)]
mod tests {
    use super::{conversation_id_from_url, derive_recovery_conversation_id};
    use crate::browser::RequestLifecycleUpdate;
    use shared_protocol::JobStatus;

    #[test]
    fn derives_recovery_conversation_id_from_latest_update_url() {
        let update = RequestLifecycleUpdate {
            status: JobStatus::Interrupted,
            message: "stalled".to_owned(),
            conversation_id: None,
            message_id: None,
            current_url: Some("https://chatgpt.com/c/abc-123?foo=bar".to_owned()),
            streaming_safe: false,
            baseline_conversation_id: None,
            baseline_message_id: None,
            baseline_assistant_message_count: None,
            baseline_text: None,
        };

        let derived = derive_recovery_conversation_id(None, Some(&update), None);

        assert_eq!(derived.as_deref(), Some("abc-123"));
    }

    #[test]
    fn extracts_conversation_id_from_chatgpt_url() {
        assert_eq!(
            conversation_id_from_url("https://chatgpt.com/c/69e4dd56-83d0-8320-a7f9-af910cb31fd1"),
            Some("69e4dd56-83d0-8320-a7f9-af910cb31fd1".to_owned())
        );
        assert_eq!(conversation_id_from_url("https://chatgpt.com/"), None);
    }
}
