use crate::browser::devtools::ExistingDebugTarget;
use crate::browser::start_debug::BrowserStartDebugPlan;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AttachedDaemonBrowser {
    pub(super) target_tab_id: Option<String>,
    pub(super) authenticated: bool,
    pub(super) extension_connected: bool,
    pub(super) cdp_attached: bool,
}

pub(super) fn attached_daemon_browser_output(
    config: &gptd::AppConfig,
    plan: &BrowserStartDebugPlan,
    attached: AttachedDaemonBrowser,
) -> Value {
    json!({
        "status": "already_running",
        "browser_path": plan.executable.clone(),
        "target_url": plan.target_url.clone(),
        "debug_endpoint": config.browser_debug_endpoint(),
        "debug_host": config.browser_debug_host(),
        "debug_port": config.browser_debug_port(),
        "attach_profile_dir": plan.attach_profile_dir.as_ref().map(|path| path.display().to_string()),
        "launched_pid": Value::Null,
        "launch_args": Value::Null,
        "browser_user_owned": true,
        "browser_owner": "external",
        "action": "reused_existing_debug_browser",
        "manual_sign_in_required": true,
        "requires_clean_browser_restart_for_fresh_debug_endpoint": true,
        "target_url_opened_or_focused": false,
        "target_tab_found_before_open": true,
        "target_tab_found": true,
        "target_tab_focused": false,
        "reused_existing_debug_endpoint": true,
        "launched_new_process": false,
        "existing_process_detected": true,
        "target_tab_id": attached.target_tab_id,
        "authenticated": attached.authenticated,
        "extension_connected": attached.extension_connected,
        "cdp_attached": attached.cdp_attached,
        "next_commands": [
            "gpt browser attach",
            "gpt status",
            "gpt doctor",
        ],
        "note": browser_start_debug_note(plan.attach_profile_dir.is_some()),
    })
}

pub(super) fn existing_debug_target_output(
    config: &gptd::AppConfig,
    plan: &BrowserStartDebugPlan,
    existing: ExistingDebugTarget,
) -> Value {
    json!({
        "status": existing.status,
        "browser_path": plan.executable.clone(),
        "target_url": plan.target_url.clone(),
        "debug_endpoint": config.browser_debug_endpoint(),
        "debug_host": config.browser_debug_host(),
        "debug_port": config.browser_debug_port(),
        "attach_profile_dir": plan.attach_profile_dir.as_ref().map(|path| path.display().to_string()),
        "launched_pid": Value::Null,
        "launch_args": Value::Null,
        "browser_user_owned": true,
        "browser_owner": "external",
        "action": existing.action,
        "manual_sign_in_required": true,
        "requires_clean_browser_restart_for_fresh_debug_endpoint": true,
        "target_url_opened_or_focused": existing.target_url_opened_or_focused,
        "target_tab_found_before_open": existing.target_tab_found_before_open,
        "target_tab_found": existing.target_tab_found,
        "target_tab_focused": existing.target_tab_focused,
        "reused_existing_debug_endpoint": true,
        "launched_new_process": false,
        "existing_process_detected": true,
        "target_tab_id": existing.target_tab_id,
        "next_commands": [
            "gpt browser attach",
            "gpt status",
            "gpt doctor",
        ],
        "note": browser_start_debug_note(plan.attach_profile_dir.is_some()),
    })
}

pub(super) fn existing_debug_browser_starting_output(
    config: &gptd::AppConfig,
    plan: &BrowserStartDebugPlan,
) -> Value {
    json!({
        "status": "already_running",
        "browser_path": plan.executable.clone(),
        "target_url": plan.target_url.clone(),
        "debug_endpoint": config.browser_debug_endpoint(),
        "debug_host": config.browser_debug_host(),
        "debug_port": config.browser_debug_port(),
        "attach_profile_dir": plan.attach_profile_dir.as_ref().map(|path| path.display().to_string()),
        "launched_pid": Value::Null,
        "launch_args": Value::Null,
        "browser_user_owned": true,
        "browser_owner": "external",
        "action": "existing_debug_browser_starting",
        "manual_sign_in_required": true,
        "requires_clean_browser_restart_for_fresh_debug_endpoint": true,
        "target_url_opened_or_focused": false,
        "target_tab_found": Value::Null,
        "reused_existing_debug_endpoint": false,
        "launched_new_process": false,
        "existing_process_detected": true,
        "target_tab_id": Value::Null,
        "next_commands": [
            "gpt browser attach",
            "gpt status",
            "gpt doctor",
        ],
        "note": browser_start_debug_note(plan.attach_profile_dir.is_some()),
    })
}

pub(super) fn started_debug_browser_output(
    config: &gptd::AppConfig,
    plan: &BrowserStartDebugPlan,
    launched_pid: u32,
) -> Value {
    json!({
        "status": "started",
        "browser_path": plan.executable.clone(),
        "target_url": plan.target_url.clone(),
        "debug_endpoint": config.browser_debug_endpoint(),
        "debug_host": config.browser_debug_host(),
        "debug_port": config.browser_debug_port(),
        "attach_profile_dir": plan.attach_profile_dir.as_ref().map(|path| path.display().to_string()),
        "launched_pid": launched_pid,
        "launch_args": plan.launch_args.clone(),
        "browser_user_owned": true,
        "browser_owner": "external",
        "action": "started_debug_browser",
        "manual_sign_in_required": true,
        "requires_clean_browser_restart_for_fresh_debug_endpoint": true,
        "target_url_opened_or_focused": true,
        "target_tab_found": Value::Null,
        "reused_existing_debug_endpoint": false,
        "launched_new_process": true,
        "existing_process_detected": false,
        "next_commands": [
            "gpt browser attach",
            "gpt status",
            "gpt doctor",
        ],
        "note": browser_start_debug_note(plan.attach_profile_dir.is_some()),
    })
}

pub(super) fn browser_start_debug_note(uses_chrome_attach_profile: bool) -> &'static str {
    if uses_chrome_attach_profile {
        "This starts Chrome in attach-first mode with a dedicated persistent real profile under the Tablinq state root so current Chrome will expose the local debug endpoint. It does not create a managed automation profile."
    } else {
        "This starts your normal browser with a local debug endpoint. It does not create a managed automation profile."
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::start_debug::build_browser_start_debug_plan;
    use std::path::Path;
    use tempfile::tempdir;

    #[test]
    fn started_debug_browser_output_preserves_json_shape() {
        let dir = tempdir().unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();
        let plan = build_browser_start_debug_plan(
            &config,
            Some("https://chatgpt.com/"),
            Some(Path::new(
                "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
            )),
        )
        .unwrap();

        let value = started_debug_browser_output(&config, &plan, 1234);

        assert_eq!(value["status"], "started");
        assert_eq!(value["action"], "started_debug_browser");
        assert_eq!(value["browser_owner"], "external");
        assert_eq!(value["browser_user_owned"], true);
        assert_eq!(value["manual_sign_in_required"], true);
        assert_eq!(value["launched_pid"], 1234);
        assert_eq!(value["target_url_opened_or_focused"], true);
        assert_eq!(value["target_tab_found"], Value::Null);
        assert_eq!(value["reused_existing_debug_endpoint"], false);
        assert_eq!(value["launched_new_process"], true);
        assert_eq!(value["existing_process_detected"], false);
        assert!(value.get("pid").is_none());
        assert!(value.get("close_existing_browser_windows_first").is_none());
    }

    #[test]
    fn attached_daemon_browser_output_preserves_reuse_shape() {
        let dir = tempdir().unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();
        let plan = build_browser_start_debug_plan(
            &config,
            Some("https://chatgpt.com/"),
            Some(Path::new(
                "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
            )),
        )
        .unwrap();

        let value = attached_daemon_browser_output(
            &config,
            &plan,
            AttachedDaemonBrowser {
                target_tab_id: Some("stable-attached-tab".to_owned()),
                authenticated: true,
                extension_connected: true,
                cdp_attached: true,
            },
        );

        assert_eq!(value["status"], "already_running");
        assert_eq!(value["action"], "reused_existing_debug_browser");
        assert_eq!(value["target_tab_id"], "stable-attached-tab");
        assert_eq!(value["target_url_opened_or_focused"], false);
        assert_eq!(value["target_tab_focused"], false);
        assert_eq!(value["launch_args"], Value::Null);
        assert_eq!(value["launched_pid"], Value::Null);
        assert_eq!(value["authenticated"], true);
        assert_eq!(value["extension_connected"], true);
        assert_eq!(value["cdp_attached"], true);
    }
}
