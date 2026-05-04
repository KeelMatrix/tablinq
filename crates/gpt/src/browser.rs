mod detection;
mod devtools;
mod output;
mod start_debug;
mod urls;

use crate::cli::BrowserCommand;
use crate::daemon::send_daemon_request;
use anyhow::Result;
use serde_json::{Value, json};
use shared_protocol::{
    BrowserConnectionMode, BrowserOpenRequest, CHROME_ATTACH_PROFILE_NOTE, DaemonRequest,
};
use std::path::Path;

pub(crate) fn run_browser_command(
    state_root: Option<&Path>,
    command: &BrowserCommand,
) -> Result<Value> {
    let config = gptd::AppConfig::load_or_create(state_root)?;
    match command {
        BrowserCommand::Open {
            target_url,
            browser_path,
            headless,
        }
        | BrowserCommand::Attach {
            target_url,
            browser_path,
            headless,
        } => send_daemon_request(
            state_root,
            DaemonRequest::BrowserOpen(BrowserOpenRequest {
                target_url: target_url.clone(),
                browser_path: browser_path.as_ref().map(|path| path.display().to_string()),
                headless: Some(*headless),
            }),
        ),
        BrowserCommand::Detach => send_daemon_request(state_root, DaemonRequest::BrowserClose),
        BrowserCommand::StartDebug {
            target_url,
            browser_path,
        } => start_debug::browser_start_debug(
            state_root,
            &config,
            target_url.as_deref(),
            browser_path.as_deref(),
        ),
        BrowserCommand::ConnectionInfo => browser_connection_info(state_root, &config),
        BrowserCommand::ProfileInfo => {
            send_daemon_request(state_root, DaemonRequest::BrowserProfileInfo)
        }
    }
}

fn browser_connection_info(state_root: Option<&Path>, config: &gptd::AppConfig) -> Result<Value> {
    let browser = if gptd::daemon_is_responsive(&config.paths.ipc_endpoint) {
        send_daemon_request(state_root, DaemonRequest::BrowserProfileInfo)?
    } else {
        Value::Null
    };
    let mode = config.browser_connection_mode();
    let startup = attach_startup_story(config);

    Ok(json!({
        "attach_mode": mode,
        "debug_endpoint": config.browser_debug_endpoint(),
        "debug_host": config.browser_debug_host(),
        "debug_port": config.browser_debug_port(),
        "browser_channel": config.browser_channel(),
        "browser_path": config.browser_path(),
        "target_url": config.browser_target_url(),
        "extension_required": config.browser_extension_required(),
        "auto_open_target_tab": config.browser_auto_open_target_tab(),
        "startup_story": startup,
        "recommended_commands": browser_recovery_commands(mode),
        "browser": browser,
    }))
}

pub(crate) fn attach_startup_story(config: &gptd::AppConfig) -> Value {
    if !is_attach_primary(config.browser_connection_mode()) {
        return json!({
            "mode": "managed_only",
            "command": "gpt browser open",
        });
    }

    let preview = match start_debug::build_browser_start_debug_plan(config, None, None) {
        Ok(plan) => json!({
            "browser_path": plan.executable,
            "launch_args": plan.launch_args,
            "attach_profile_dir": plan.attach_profile_dir.as_ref().map(|path| path.display().to_string()),
            "profile_note": plan.attach_profile_dir.as_ref().map(|_| CHROME_ATTACH_PROFILE_NOTE),
        }),
        Err(error) => json!({
            "browser_path": Value::Null,
            "launch_error": error.to_string(),
        }),
    };

    json!({
        "mode": "start_debug",
        "command": "gpt browser start-debug",
        "browser_user_owned": true,
        "manual_sign_in_required": true,
        "requires_clean_browser_restart_for_fresh_debug_endpoint": true,
        "extension_install_required": config.browser_extension_required(),
        "verify_commands": [
            "gpt browser attach",
            "gpt status",
            "gpt doctor",
            "gpt browser connection-info",
        ],
        "fallback": {
            "mode": "managed_only",
            "config_toml": "browser_connection_mode = \"managed_only\"",
            "command": "gpt browser open",
            "when_to_use": "Use managed_only only for debugging, CI/local harness isolation, or when your normal browser cannot expose the local debug endpoint."
        },
        "preview": preview,
    })
}

pub(crate) fn browser_recovery_commands(mode: BrowserConnectionMode) -> Vec<&'static str> {
    let mut commands = vec!["gpt daemon start", "gpt gateway start"];
    if is_attach_primary(mode) {
        commands.push("gpt browser start-debug");
        commands.push("gpt browser connection-info");
        commands.push("gpt browser attach");
        commands.push("gpt browser open");
    } else {
        commands.push("gpt browser open");
    }
    commands.push("gpt gateway stop");
    commands.push("gpt daemon stop");
    commands
}

pub(crate) fn is_attach_primary(mode: BrowserConnectionMode) -> bool {
    matches!(
        mode,
        BrowserConnectionMode::AttachFirst | BrowserConnectionMode::ManagedFallback
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, run};
    use clap::Parser;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn browser_connection_info_reports_attach_first_defaults() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("config")).unwrap();
        fs::write(
            dir.path().join("config").join("config.toml"),
            "version = 2\nbrowser_channel = \"chrome\"\n",
        )
        .unwrap();
        let cli = Cli::parse_from([
            "gpt",
            "--state-root",
            dir.path().to_str().unwrap(),
            "browser",
            "connection-info",
        ]);

        let output = run(&cli).unwrap();

        assert!(output.contains("\"attach_mode\": \"attach_first\""));
        assert!(output.contains("\"debug_endpoint\": \"http://127.0.0.1:9222\""));
        assert!(output.contains("\"gpt browser start-debug\""));
        assert!(output.contains("\"gpt browser attach\""));
        assert!(output.contains("\"browser_connection_mode = \\\"managed_only\\\"\""));
        assert!(output.contains("\"attach_profile_dir\""));
        assert!(output.contains(CHROME_ATTACH_PROFILE_NOTE));
        assert!(
            output.contains("\"requires_clean_browser_restart_for_fresh_debug_endpoint\": true")
        );
        assert!(!output.contains("close_existing_browser_windows_first"));
    }

    #[test]
    fn startup_story_preserves_start_debug_shape() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("config")).unwrap();
        fs::write(
            dir.path().join("config").join("config.toml"),
            "version = 2\nbrowser_channel = \"chrome\"\nbrowser_path = \"C:\\\\Chrome\\\\chrome.exe\"\n",
        )
        .unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();

        let story = attach_startup_story(&config);

        assert_eq!(story["mode"], "start_debug");
        assert_eq!(story["command"], "gpt browser start-debug");
        assert_eq!(story["browser_user_owned"], true);
        assert_eq!(story["manual_sign_in_required"], true);
        assert_eq!(
            story["requires_clean_browser_restart_for_fresh_debug_endpoint"],
            true
        );
        assert_eq!(story["extension_install_required"], true);
        assert_eq!(story["preview"]["browser_path"], "C:\\Chrome\\chrome.exe");
        assert_eq!(
            story["preview"]["attach_profile_dir"],
            config.paths.chrome_attach_profile_dir.display().to_string()
        );
        assert_eq!(story["preview"]["profile_note"], CHROME_ATTACH_PROFILE_NOTE);
        assert_eq!(story["fallback"]["mode"], "managed_only");
        assert_eq!(story["fallback"]["command"], "gpt browser open");
    }
}
