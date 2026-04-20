use crate::cli::BrowserCommand;
use crate::daemon::send_daemon_request;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use shared_protocol::{
    BrowserConnectionMode, BrowserOpenRequest, CHROME_ATTACH_PROFILE_NOTE, DaemonRequest,
};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

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
        BrowserCommand::Close => send_daemon_request(state_root, DaemonRequest::BrowserClose),
        BrowserCommand::Logout | BrowserCommand::SwitchAccount => {
            send_daemon_request(state_root, DaemonRequest::BrowserLogout)
        }
        BrowserCommand::ResetProfile => {
            if !supports_managed_profile_reset(config.browser_connection_mode()) {
                bail!(
                    "browser reset-profile is only available in managed_fallback or managed_only; current mode is attach_first"
                );
            }
            send_daemon_request(state_root, DaemonRequest::BrowserResetProfile)
        }
        BrowserCommand::StartDebug {
            target_url,
            browser_path,
        } => browser_start_debug(&config, target_url.as_deref(), browser_path.as_deref()),
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserStartDebugPlan {
    executable: String,
    launch_args: Vec<String>,
    target_url: String,
    attach_profile_dir: Option<PathBuf>,
}

fn browser_start_debug(
    config: &gptd::AppConfig,
    target_url: Option<&str>,
    browser_path: Option<&Path>,
) -> Result<Value> {
    let plan = build_browser_start_debug_plan(config, target_url, browser_path)?;
    if let Some(attach_profile_dir) = plan.attach_profile_dir.as_ref() {
        fs::create_dir_all(attach_profile_dir)
            .with_context(|| format!("create {}", attach_profile_dir.display()))?;
    }
    let mut child = ProcessCommand::new(&plan.executable);
    child
        .args(&plan.launch_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = child.spawn().with_context(|| {
        format!(
            "start browser in debug attach mode with executable {}",
            plan.executable
        )
    })?;

    Ok(json!({
        "status": "started",
        "browser_path": plan.executable,
        "target_url": plan.target_url,
        "debug_endpoint": config.browser_debug_endpoint(),
        "debug_host": config.browser_debug_host(),
        "debug_port": config.browser_debug_port(),
        "attach_profile_dir": plan.attach_profile_dir.as_ref().map(|path| path.display().to_string()),
        "pid": child.id(),
        "launch_args": plan.launch_args,
        "browser_user_owned": true,
        "manual_sign_in_required": true,
        "close_existing_browser_windows_first": true,
        "next_commands": [
            "gpt browser attach",
            "gpt status",
            "gpt doctor",
        ],
        "note": browser_start_debug_note(plan.attach_profile_dir.is_some()),
    }))
}

fn build_browser_start_debug_plan(
    config: &gptd::AppConfig,
    target_url: Option<&str>,
    browser_path: Option<&Path>,
) -> Result<BrowserStartDebugPlan> {
    let executable = resolve_browser_executable(config, browser_path)?;
    let target_url = target_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(config.browser_target_url())
        .to_owned();
    let attach_profile_dir =
        chrome_attach_profile_dir_for_start_debug(config, browser_path, &executable);
    let mut launch_args = vec![
        format!("--remote-debugging-port={}", config.browser_debug_port()),
        format!("--remote-debugging-address={}", config.browser_debug_host()),
    ];
    if let Some(attach_profile_dir) = attach_profile_dir.as_ref() {
        launch_args.push(format!("--user-data-dir={}", attach_profile_dir.display()));
    }
    launch_args.extend([
        "--no-first-run".to_owned(),
        "--no-default-browser-check".to_owned(),
        target_url.clone(),
    ]);

    Ok(BrowserStartDebugPlan {
        executable,
        launch_args,
        target_url,
        attach_profile_dir,
    })
}

fn browser_start_debug_note(uses_chrome_attach_profile: bool) -> &'static str {
    if uses_chrome_attach_profile {
        "This starts Chrome in attach-first mode with a dedicated persistent real profile under the Tablinq state root so current Chrome will expose the local debug endpoint. It does not create a managed automation profile."
    } else {
        "This starts your normal browser with a local debug endpoint. It does not create a managed automation profile."
    }
}

fn chrome_attach_profile_dir_for_start_debug(
    config: &gptd::AppConfig,
    browser_path: Option<&Path>,
    executable: &str,
) -> Option<PathBuf> {
    if uses_chrome_attach_profile(config, browser_path, executable) {
        Some(config.paths.chrome_attach_profile_dir.clone())
    } else {
        None
    }
}

fn uses_chrome_attach_profile(
    config: &gptd::AppConfig,
    browser_path: Option<&Path>,
    executable: &str,
) -> bool {
    if executable_looks_like_edge(executable) {
        return false;
    }
    if executable_looks_like_chrome(executable) {
        return true;
    }

    let has_explicit_browser_path = browser_path.is_some() || config.browser_path().is_some();
    !has_explicit_browser_path && config.browser_channel().eq_ignore_ascii_case("chrome")
}

fn executable_looks_like_edge(executable: &str) -> bool {
    let lower = executable.to_ascii_lowercase();
    lower.contains("msedge") || lower.contains("microsoft edge")
}

fn executable_looks_like_chrome(executable: &str) -> bool {
    let lower = executable.to_ascii_lowercase();
    lower.contains("google chrome") || lower.contains("chrome.exe") || lower.contains("chrome")
}

fn resolve_browser_executable(
    config: &gptd::AppConfig,
    browser_path: Option<&Path>,
) -> Result<String> {
    if let Some(browser_path) = browser_path {
        return Ok(browser_path.display().to_string());
    }
    if let Some(browser_path) = config.browser_path() {
        return Ok(browser_path.to_owned());
    }
    if let Ok(browser_path) = env::var("TABLINQ_BROWSER_PATH") {
        let browser_path = browser_path.trim();
        if !browser_path.is_empty() {
            return Ok(browser_path.to_owned());
        }
    }

    detect_browser_executable(config.browser_channel()).ok_or_else(|| {
        anyhow::anyhow!(
            "no supported browser executable found for channel {}; set browser_path in config or pass --browser-path",
            config.browser_channel()
        )
    })
}

fn detect_browser_executable(channel: &str) -> Option<String> {
    if cfg!(windows) {
        detect_windows_browser_executable(channel)
    } else if cfg!(target_os = "macos") {
        let mut candidates = ordered_browser_candidates(
            channel,
            &[
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            ],
        );
        candidates.retain(|candidate| Path::new(candidate).exists());
        candidates.into_iter().next().map(str::to_owned)
    } else {
        ordered_browser_candidates(
            channel,
            &[
                "google-chrome",
                "chromium",
                "chromium-browser",
                "microsoft-edge",
            ],
        )
        .into_iter()
        .find_map(resolve_command_in_path)
    }
}

fn ordered_browser_candidates<'a>(channel: &str, candidates: &'a [&'a str]) -> Vec<&'a str> {
    let (preferred, fallback) = if channel.eq_ignore_ascii_case("msedge") {
        (
            vec!["microsoft-edge", "msedge.exe"],
            vec!["google chrome", "chrome.exe"],
        )
    } else {
        (
            vec!["google chrome", "chrome.exe"],
            vec!["microsoft-edge", "msedge.exe"],
        )
    };

    let mut ordered = Vec::new();
    for candidate in candidates {
        let lower = candidate.to_ascii_lowercase();
        if preferred.iter().any(|value| lower.contains(value)) {
            ordered.push(*candidate);
        }
    }
    for candidate in candidates {
        let lower = candidate.to_ascii_lowercase();
        if fallback.iter().any(|value| lower.contains(value)) && !ordered.contains(candidate) {
            ordered.push(*candidate);
        }
    }
    for candidate in candidates {
        if !ordered.contains(candidate) {
            ordered.push(*candidate);
        }
    }
    ordered
}

fn resolve_command_in_path(command: &str) -> Option<String> {
    let tool = if cfg!(windows) { "where" } else { "which" };
    ProcessCommand::new(tool)
        .arg(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8(output.stdout).ok().and_then(|value| {
                value
                    .lines()
                    .map(str::trim)
                    .find(|value| !value.is_empty())
                    .map(str::to_owned)
            })
        })
}

fn detect_windows_browser_executable(channel: &str) -> Option<String> {
    let local_app_data = env::var("LOCALAPPDATA")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let program_files = [
        env::var("ProgramFiles(x86)").ok(),
        env::var("ProgramFiles").ok(),
    ]
    .into_iter()
    .flatten()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>();

    let chrome_candidates = std::iter::once(local_app_data.as_deref().map(|base| {
        Path::new(base)
            .join("Google")
            .join("Chrome")
            .join("Application")
            .join("chrome.exe")
    }))
    .flatten()
    .chain(program_files.iter().map(|base| {
        Path::new(base)
            .join("Google")
            .join("Chrome")
            .join("Application")
            .join("chrome.exe")
    }))
    .collect::<Vec<_>>();
    let edge_candidates = std::iter::once(local_app_data.as_deref().map(|base| {
        Path::new(base)
            .join("Microsoft")
            .join("Edge")
            .join("Application")
            .join("msedge.exe")
    }))
    .flatten()
    .chain(program_files.iter().map(|base| {
        Path::new(base)
            .join("Microsoft")
            .join("Edge")
            .join("Application")
            .join("msedge.exe")
    }))
    .collect::<Vec<_>>();

    let ordered_paths = if channel.eq_ignore_ascii_case("msedge") {
        edge_candidates
            .into_iter()
            .chain(chrome_candidates)
            .collect::<Vec<_>>()
    } else {
        chrome_candidates
            .into_iter()
            .chain(edge_candidates)
            .collect::<Vec<_>>()
    };
    for candidate in ordered_paths {
        if candidate.exists() {
            return Some(candidate.display().to_string());
        }
    }

    let ordered_commands = if channel.eq_ignore_ascii_case("msedge") {
        vec!["msedge.exe", "chrome.exe"]
    } else {
        vec!["chrome.exe", "msedge.exe"]
    };
    ordered_commands
        .into_iter()
        .find_map(resolve_command_in_path)
}

pub(crate) fn attach_startup_story(config: &gptd::AppConfig) -> Value {
    if !is_attach_primary(config.browser_connection_mode()) {
        return json!({
            "mode": "managed_only",
            "command": "gpt browser open",
        });
    }

    let preview = match build_browser_start_debug_plan(config, None, None) {
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
        "close_existing_browser_windows_first": true,
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
        commands.push("gpt browser reset-profile");
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

pub(crate) fn supports_managed_profile_reset(mode: BrowserConnectionMode) -> bool {
    matches!(
        mode,
        BrowserConnectionMode::ManagedFallback | BrowserConnectionMode::ManagedOnly
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, run};
    use clap::Parser;
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
    }

    #[test]
    fn browser_start_debug_serialized_output_includes_chrome_attach_profile_fields() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("config")).unwrap();
        fs::write(
            dir.path().join("config").join("config.toml"),
            "version = 2\nbrowser_channel = \"chrome\"\n",
        )
        .unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();
        let browser_path = create_fake_browser_launcher(dir.path(), "chrome");

        let value = browser_start_debug(&config, Some("https://chatgpt.com/"), Some(&browser_path))
            .unwrap();

        let attach_profile_dir = value
            .get("attach_profile_dir")
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(
            attach_profile_dir,
            config.paths.chrome_attach_profile_dir.display().to_string()
        );
        assert_eq!(
            value.get("note").and_then(Value::as_str),
            Some(browser_start_debug_note(true))
        );
        assert!(
            value["launch_args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item.as_str() == Some(&format!("--user-data-dir={attach_profile_dir}")))
        );
        assert!(config.paths.chrome_attach_profile_dir.exists());
    }

    #[test]
    fn start_debug_plan_uses_dedicated_chrome_attach_profile() {
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

        assert_eq!(
            plan.executable,
            "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe"
        );
        assert!(
            plan.launch_args
                .contains(&"--remote-debugging-port=9222".to_owned())
        );
        assert!(
            plan.launch_args
                .contains(&"--remote-debugging-address=127.0.0.1".to_owned())
        );
        assert_eq!(
            plan.attach_profile_dir,
            Some(config.paths.chrome_attach_profile_dir.clone())
        );
        assert_eq!(
            config.paths.chrome_attach_profile_dir,
            dir.path().join("chrome-attach-profile")
        );
        assert!(
            config
                .paths
                .chrome_attach_profile_dir
                .starts_with(dir.path())
        );
        assert!(plan.launch_args.contains(&format!(
            "--user-data-dir={}",
            config.paths.chrome_attach_profile_dir.display()
        )));
        assert_ne!(
            config.paths.chrome_attach_profile_dir,
            config.paths.browser_profile_dir
        );
    }

    #[test]
    fn start_debug_plan_leaves_edge_without_attach_profile_flag() {
        let dir = tempdir().unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();

        let plan = build_browser_start_debug_plan(
            &config,
            Some("https://chatgpt.com/"),
            Some(Path::new("C:\\Browsers\\Edge\\msedge.exe")),
        )
        .unwrap();

        assert_eq!(plan.executable, "C:\\Browsers\\Edge\\msedge.exe");
        assert_eq!(plan.attach_profile_dir, None);
        assert!(
            !plan
                .launch_args
                .iter()
                .any(|arg| arg.contains("--user-data-dir"))
        );
    }

    fn create_fake_browser_launcher(root: &Path, name: &str) -> PathBuf {
        #[cfg(windows)]
        let path = root.join(format!("{name}.cmd"));
        #[cfg(not(windows))]
        let path = root.join(name);

        #[cfg(windows)]
        fs::write(&path, "@echo off\r\nexit /b 0\r\n").unwrap();

        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).unwrap();
        }

        path
    }

    #[test]
    fn reset_profile_is_rejected_in_attach_first_mode() {
        let dir = tempdir().unwrap();
        let cli = Cli::parse_from([
            "gpt",
            "--state-root",
            dir.path().to_str().unwrap(),
            "browser",
            "reset-profile",
        ]);

        let err = run(&cli).unwrap_err();
        assert!(format!("{err:#}").contains("managed_fallback or managed_only"));
    }
}
