use crate::browser::{attach_startup_story, browser_recovery_commands, is_attach_primary};
use crate::daemon::send_daemon_request;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use shared_protocol::{BrowserConnectionMode, DaemonRequest};
use std::path::Path;

pub(crate) fn config_show(state_root: Option<&Path>) -> Result<Value> {
    let config = gptd::AppConfig::load_or_create(state_root)?;
    serde_json::to_value(config.snapshot()).context("serialize config snapshot")
}

pub(crate) fn operational_report(state_root: Option<&Path>) -> Result<Value> {
    let config = gptd::AppConfig::load_or_create(state_root)?;
    let connection_mode = config.browser_connection_mode();
    let config_snapshot =
        serde_json::to_value(config.snapshot()).context("serialize config snapshot")?;
    let state_root_path = config.paths.state_root.display().to_string();
    let ipc_endpoint = config.paths.ipc_endpoint.clone();
    let logs_dir = config.paths.logs_dir.display().to_string();
    let daemon_log = config.paths.log_path.display().to_string();
    let gateway_log = config
        .paths
        .logs_dir
        .join("gateway.log")
        .display()
        .to_string();
    let daemon_running = gptd::daemon_is_responsive(&config.paths.ipc_endpoint);
    let daemon_doctor = if daemon_running {
        Some(send_daemon_request(state_root, DaemonRequest::Doctor)?)
    } else {
        None
    };
    let daemon_doctor_payload = daemon_doctor
        .as_ref()
        .and_then(|value| daemon_response_payload(value, "doctor"));
    let gateway = tablinq_gateway::gateway_doctor(state_root)?;
    let health = daemon_doctor_payload
        .and_then(|value| value.get("health"))
        .cloned()
        .or_else(|| {
            gateway
                .get("health")
                .and_then(|value| value.get("health"))
                .cloned()
        })
        .unwrap_or(Value::Null);
    let browser = daemon_doctor_payload
        .and_then(|value| value.get("browser"))
        .cloned()
        .unwrap_or(Value::Null);
    let connectivity = resolved_browser_connectivity(&browser, &health);
    let extension_connected = connectivity.extension_connected;
    let authenticated = connectivity.authenticated;
    let browser_connected = connectivity.browser_connected;
    let queue = gateway.get("scheduler").cloned().unwrap_or(Value::Null);
    let browser_state = browser_runtime_state(
        connection_mode,
        browser.as_object(),
        browser_connected,
        extension_connected,
        authenticated,
    );
    let recovery_guidance =
        operational_recovery_guidance(&config, browser_state, daemon_doctor_payload);

    Ok(json!({
        "service": "tablinq",
        "state_root": state_root_path,
        "config": config_snapshot,
        "daemon": {
            "running": daemon_running,
            "ipc_endpoint": ipc_endpoint,
            "doctor": daemon_doctor_payload.cloned(),
        },
        "gateway": gateway,
        "browser": {
            "state": browser_state,
            "connected": browser_connected,
            "snapshot": browser,
            "startup_story": attach_startup_story(&config),
        },
        "extension": {
            "connected": extension_connected,
        },
        "auth": {
            "authenticated": authenticated,
        },
        "queue": queue,
        "logs": {
            "directory": logs_dir,
            "daemon": daemon_log,
            "gateway": gateway_log,
        },
        "safe_parallelism": {
            "max_parallel_requests": 1,
            "max_parallel_branches": 1,
            "max_subagents": 0,
        },
        "recovery_guidance": recovery_guidance,
        "recovery_commands": browser_recovery_commands(connection_mode),
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BrowserConnectivity {
    browser_connected: bool,
    extension_connected: bool,
    authenticated: bool,
}

fn resolved_browser_connectivity(browser: &Value, health: &Value) -> BrowserConnectivity {
    let browser_connected = browser
        .get("running")
        .and_then(Value::as_bool)
        .or_else(|| health.get("browser_connected").and_then(Value::as_bool))
        .unwrap_or(false);
    let extension_connected = browser
        .get("extension_connected")
        .and_then(Value::as_bool)
        .or_else(|| health.get("extension_connected").and_then(Value::as_bool))
        .unwrap_or(false);
    let authenticated = browser
        .get("authenticated")
        .and_then(Value::as_bool)
        .or_else(|| health.get("authenticated").and_then(Value::as_bool))
        .unwrap_or(false);

    BrowserConnectivity {
        browser_connected,
        extension_connected,
        authenticated,
    }
}

fn daemon_response_payload<'a>(value: &'a Value, response_type: &str) -> Option<&'a Value> {
    if value.get("type").and_then(Value::as_str) == Some(response_type) {
        value.get("payload")
    } else {
        None
    }
}

fn browser_runtime_state(
    mode: BrowserConnectionMode,
    browser: Option<&serde_json::Map<String, Value>>,
    browser_connected: bool,
    extension_connected: bool,
    authenticated: bool,
) -> &'static str {
    let target_tab_found = browser
        .and_then(|value| value.get("target_tab_found"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let browser_owner = browser
        .and_then(|value| value.get("browser_owner"))
        .and_then(Value::as_str)
        .unwrap_or(if is_attach_primary(mode) {
            "external"
        } else {
            "managed"
        });
    let action = browser
        .and_then(|value| value.get("action"))
        .and_then(Value::as_str);

    if browser_owner == "external" || (is_attach_primary(mode) && !browser_connected) {
        if !browser_connected && action == Some("detached") {
            "attach_detached"
        } else if !browser_connected {
            "attach_unavailable"
        } else if !target_tab_found {
            "target_tab_missing"
        } else if !authenticated {
            "attached_signed_out"
        } else if !extension_connected {
            "attached_extension_disconnected"
        } else {
            "attached_ready"
        }
    } else if !browser_connected {
        "managed_not_started"
    } else if !authenticated {
        "managed_signed_out"
    } else if !extension_connected {
        "managed_extension_disconnected"
    } else {
        "managed_ready"
    }
}

fn operational_recovery_guidance(
    config: &gptd::AppConfig,
    browser_state: &str,
    daemon_doctor: Option<&Value>,
) -> Value {
    if let Some(guidance) = daemon_doctor.and_then(|value| value.get("recovery_guidance")) {
        return guidance.clone();
    }

    if is_attach_primary(config.browser_connection_mode()) {
        let guidance = match browser_state {
            "attach_detached" => vec![
                "Tablinq is detached from the attach-first browser session.".to_owned(),
                format!(
                    "If {} is still open, run `gpt browser attach`.",
                    recovery_browser_label(config)
                ),
                format!(
                    "If attach fails or the debug endpoint is gone, fully close {} and run `gpt browser start-debug`.",
                    recovery_browser_label(config)
                ),
            ],
            "attach_unavailable" => vec![
                format!(
                    "check the attach endpoint at {}; fully close {}, run `gpt browser start-debug`, then inspect `gpt browser connection-info`",
                    config.browser_debug_endpoint(),
                    recovery_browser_label(config)
                ),
                "confirm the Tablinq extension is installed and enabled in the same normal browser profile".to_owned(),
                "sign in to ChatGPT in that browser and clear any login or challenge page".to_owned(),
                "run `gpt browser attach` and then `gpt browser open` to reattach or focus the configured target tab".to_owned(),
                "only if the local debug endpoint cannot be enabled on this machine, or you need debugging or CI/local harness isolation, set `browser_connection_mode = \"managed_only\"` in config.toml and use `gpt browser open` as fallback".to_owned(),
            ],
            "attached_extension_disconnected" => vec![
                "confirm the Tablinq extension is installed, enabled, and connected in the attached browser profile".to_owned(),
                "if multiple ChatGPT tabs are open, keep only the signed-in tab with the working Tablinq extension, then rerun `gpt browser attach`".to_owned(),
                "only if attach mode cannot be stabilized on this machine, or you need debugging or CI/local harness isolation, set `browser_connection_mode = \"managed_only\"` in config.toml and use `gpt browser open` as fallback".to_owned(),
            ],
            "attached_signed_out" => vec![
                "sign in to ChatGPT in the attached normal browser tab and clear any login or challenge page".to_owned(),
                "rerun `gpt browser attach` and `gpt browser open` after the signed-in tab is visible".to_owned(),
                "if multiple ChatGPT tabs are open, keep only the signed-in tab you want Tablinq to use".to_owned(),
            ],
            "target_tab_missing" => vec![
                "open a signed-in ChatGPT tab in your normal browser or run `gpt browser open` to create or focus one".to_owned(),
                "rerun `gpt browser attach` after the target tab is visible".to_owned(),
                "if multiple ChatGPT tabs are open, close extras before retrying attach".to_owned(),
            ],
            _ => vec!["attach-first browser session is ready".to_owned()],
        };
        return json!(guidance);
    }

    json!([
        "managed mode is fallback/debug-only; use `gpt browser open` to start the managed fallback browser tab",
        "sign in to ChatGPT in the managed fallback tab before running ask or research",
        "use managed mode for debugging, CI/local harness isolation, or when attach-first cannot expose a local debug endpoint",
        "if the managed browser profile is wedged, run `gpt browser reset-profile`",
    ])
}

fn recovery_browser_label(config: &gptd::AppConfig) -> &'static str {
    let channel = config.browser_channel().to_ascii_lowercase();
    if channel == "msedge" || channel == "edge" {
        "Edge"
    } else {
        "Chrome"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, run};
    use clap::Parser;
    use tempfile::tempdir;

    #[test]
    fn config_show_command_renders_pretty_json() {
        let dir = tempdir().unwrap();
        let cli = Cli::parse_from([
            "gpt",
            "--state-root",
            dir.path().to_str().unwrap(),
            "config",
            "show",
        ]);

        let output = run(&cli).unwrap();

        assert!(output.contains("\"state_root\""));
        assert!(output.contains("\"config_path\""));
        assert!(output.contains("\"browser_connection_mode\": \"attach_first\""));
        assert!(output.contains("\"browser_debug_port\": 9222"));
        assert!(output.contains("\"chrome_attach_profile_dir\""));
    }

    #[test]
    fn doctor_command_renders_operational_report_without_running_services() {
        let dir = tempdir().unwrap();
        let cli = Cli::parse_from([
            "gpt",
            "--state-root",
            dir.path().to_str().unwrap(),
            "doctor",
        ]);

        let output = run(&cli).unwrap();

        assert!(output.contains("\"config\""));
        assert!(output.contains("\"daemon\""));
        assert!(output.contains("\"gateway\""));
        assert!(output.contains("\"logs\""));
        assert!(output.contains("\"browser_connection_mode\": \"attach_first\""));
        assert!(output.contains("\"state\": \"attach_unavailable\""));
        assert!(output.contains("\"recovery_guidance\""));
        assert!(output.contains("check the attach endpoint"));
        assert!(output.contains("confirm the Tablinq extension is installed"));
        assert!(output.contains("sign in to ChatGPT in that browser"));
        assert!(output.contains("\"recovery_commands\""));
        assert!(output.contains("\"gpt browser start-debug\""));
        assert!(output.contains("\"gpt browser connection-info\""));
        assert!(output.contains("\"gpt browser attach\""));
        assert!(output.contains("\"browser_connection_mode = \\\"managed_only\\\"\""));
        assert!(!output.contains("\"gpt browser reset-profile\""));
        let parsed: Value = serde_json::from_str(&output).unwrap();
        let commands = parsed
            .get("recovery_commands")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(
            commands[3],
            Value::String("gpt browser connection-info".to_owned())
        );
        assert_eq!(commands[4], Value::String("gpt browser attach".to_owned()));
    }

    #[test]
    fn status_command_renders_attach_first_recovery_commands_without_running_services() {
        let dir = tempdir().unwrap();
        let cli = Cli::parse_from([
            "gpt",
            "--state-root",
            dir.path().to_str().unwrap(),
            "status",
        ]);

        let output = run(&cli).unwrap();

        assert!(output.contains("\"state\": \"attach_unavailable\""));
        assert!(output.contains("\"recovery_guidance\""));
        assert!(output.contains("\"gpt browser start-debug\""));
        assert!(output.contains("\"gpt browser connection-info\""));
        assert!(output.contains("\"gpt browser attach\""));
        let parsed: Value = serde_json::from_str(&output).unwrap();
        let commands = parsed
            .get("recovery_commands")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(
            commands[3],
            Value::String("gpt browser connection-info".to_owned())
        );
        assert_eq!(commands[4], Value::String("gpt browser attach".to_owned()));
    }

    #[test]
    fn daemon_response_payload_unwraps_attached_doctor_health() {
        let response = json!({
            "type": "doctor",
            "payload": {
                "health": {
                    "browser_connected": true,
                    "extension_connected": true,
                    "authenticated": true,
                },
                "browser": {
                    "browser_owner": "external",
                    "target_tab_found": true,
                }
            }
        });

        let payload = daemon_response_payload(&response, "doctor").unwrap();
        let health = payload.get("health").unwrap();
        let browser = payload.get("browser").and_then(Value::as_object).unwrap();
        let browser_state = browser_runtime_state(
            BrowserConnectionMode::AttachFirst,
            Some(browser),
            health["browser_connected"].as_bool().unwrap(),
            health["extension_connected"].as_bool().unwrap(),
            health["authenticated"].as_bool().unwrap(),
        );

        assert_eq!(browser_state, "attached_ready");
    }

    #[test]
    fn resolved_browser_connectivity_prefers_live_browser_snapshot() {
        let browser = json!({
            "running": true,
            "extension_connected": true,
            "authenticated": true,
            "browser_owner": "external",
            "target_tab_found": true
        });
        let stale_health = json!({
            "browser_connected": false,
            "extension_connected": false,
            "authenticated": false
        });

        let connectivity = resolved_browser_connectivity(&browser, &stale_health);

        assert_eq!(
            connectivity,
            BrowserConnectivity {
                browser_connected: true,
                extension_connected: true,
                authenticated: true,
            }
        );
    }
}
