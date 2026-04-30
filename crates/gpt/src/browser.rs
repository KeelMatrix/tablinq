use crate::cli::BrowserCommand;
use crate::daemon::send_daemon_request;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use shared_protocol::{
    BrowserConnectionMode, BrowserOpenRequest, CHROME_ATTACH_PROFILE_NOTE, DaemonRequest,
};
use std::env;
use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::time::Duration;
use url::Url;

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
        } => browser_start_debug(
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserStartDebugPlan {
    executable: String,
    launch_args: Vec<String>,
    target_url: String,
    attach_profile_dir: Option<PathBuf>,
}

fn browser_start_debug(
    state_root: Option<&Path>,
    config: &gptd::AppConfig,
    target_url: Option<&str>,
    browser_path: Option<&Path>,
) -> Result<Value> {
    let plan = build_browser_start_debug_plan(config, target_url, browser_path)?;
    if let Some(attached) = attached_daemon_browser(state_root, config, &plan.target_url) {
        return Ok(attached_daemon_browser_output(config, &plan, attached));
    }
    if let Some(existing) = focus_existing_debug_target_before_launch(config, &plan.target_url) {
        return Ok(existing_debug_target_output(config, &plan, existing));
    }
    if attach_profile_process_running(plan.attach_profile_dir.as_deref()) {
        if let Some(existing) = wait_for_existing_debug_target(config, &plan.target_url) {
            return Ok(existing_debug_target_output(config, &plan, existing));
        }
        return Ok(json!({
            "status": "already_running",
            "browser_path": plan.executable,
            "target_url": plan.target_url,
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
        }));
    }
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
        "launched_pid": child.id(),
        "launch_args": plan.launch_args,
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
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AttachedDaemonBrowser {
    target_tab_id: Option<String>,
    authenticated: bool,
    extension_connected: bool,
    cdp_attached: bool,
}

fn attached_daemon_browser(
    state_root: Option<&Path>,
    config: &gptd::AppConfig,
    target_url: &str,
) -> Option<AttachedDaemonBrowser> {
    if !gptd::daemon_is_responsive(&config.paths.ipc_endpoint) {
        return None;
    }
    let value = send_daemon_request(state_root, DaemonRequest::BrowserProfileInfo).ok()?;
    let payload = value.get("payload")?;
    let running = payload.get("running").and_then(Value::as_bool)?;
    let cdp_attached = payload.get("cdp_attached").and_then(Value::as_bool)?;
    let target_tab_found = payload.get("target_tab_found").and_then(Value::as_bool)?;
    if !running || !cdp_attached || !target_tab_found {
        return None;
    }
    let attached_url = payload
        .get("current_url")
        .and_then(Value::as_str)
        .or_else(|| payload.get("target_url").and_then(Value::as_str))?;
    if !same_target_url(attached_url, target_url) {
        return None;
    }
    Some(AttachedDaemonBrowser {
        target_tab_id: payload
            .get("target_tab_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        authenticated: payload
            .get("authenticated")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        extension_connected: payload
            .get("extension_connected")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        cdp_attached,
    })
}

fn attached_daemon_browser_output(
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

fn existing_debug_target_output(
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExistingDebugTarget {
    status: &'static str,
    action: &'static str,
    target_url_opened_or_focused: bool,
    target_tab_found_before_open: bool,
    target_tab_found: bool,
    target_tab_focused: bool,
    target_tab_id: Option<String>,
}

fn focus_existing_debug_target(
    config: &gptd::AppConfig,
    target_url: &str,
) -> Option<ExistingDebugTarget> {
    let endpoint = parse_http_endpoint(&config.browser_debug_endpoint())?;
    let list = devtools_request(&endpoint, "GET", "/json/list").ok()?;
    let targets: Value = serde_json::from_str(&list).ok()?;
    let targets = targets.as_array()?;
    let target_id = targets.iter().find_map(|target| {
        let url = target.get("url").and_then(Value::as_str)?;
        if !same_target_url(url, target_url) {
            return None;
        }
        target
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .map(ToOwned::to_owned)
    });

    if let Some(target_id) = target_id {
        let focused =
            devtools_request(&endpoint, "GET", &format!("/json/activate/{target_id}")).is_ok();
        return Some(ExistingDebugTarget {
            status: "already_running",
            action: "reused_existing_debug_browser",
            target_url_opened_or_focused: focused,
            target_tab_found_before_open: true,
            target_tab_found: true,
            target_tab_focused: focused,
            target_tab_id: Some(target_id),
        });
    }

    let created_target_id =
        create_debug_target(&endpoint, target_url).and_then(|value| target_id_from_value(&value));
    Some(ExistingDebugTarget {
        status: "already_running",
        action: "opened_target_in_existing_debug_browser",
        target_url_opened_or_focused: created_target_id.is_some(),
        target_tab_found_before_open: false,
        target_tab_found: created_target_id.is_some(),
        target_tab_focused: false,
        target_tab_id: created_target_id,
    })
}

fn wait_for_existing_debug_target(
    config: &gptd::AppConfig,
    target_url: &str,
) -> Option<ExistingDebugTarget> {
    for _ in 0..30 {
        if let Some(existing) = focus_existing_debug_target(config, target_url) {
            return Some(existing);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

fn focus_existing_debug_target_before_launch(
    config: &gptd::AppConfig,
    target_url: &str,
) -> Option<ExistingDebugTarget> {
    for attempt in 0..5 {
        if let Some(existing) = focus_existing_debug_target(config, target_url) {
            return Some(existing);
        }
        if attempt < 4 {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    None
}

fn attach_profile_process_running(attach_profile_dir: Option<&Path>) -> bool {
    let Some(attach_profile_dir) = attach_profile_dir else {
        return false;
    };
    if cfg!(windows) {
        return command_lines_include_attach_profile(
            windows_process_command_lines(),
            attach_profile_dir,
        );
    }
    false
}

fn command_lines_include_attach_profile<I>(command_lines: I, attach_profile_dir: &Path) -> bool
where
    I: IntoIterator,
    I::Item: AsRef<str>,
{
    let profile = normalize_process_arg_path(attach_profile_dir);
    if profile.is_empty() {
        return false;
    }
    command_lines
        .into_iter()
        .map(|line| normalize_process_text(line.as_ref()))
        .any(|line| {
            line.contains("--user-data-dir") && line.contains(&profile) && line.contains("chrome")
        })
}

fn windows_process_command_lines() -> Vec<String> {
    let output = ProcessCommand::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_Process -Filter \"Name = 'chrome.exe'\" | ForEach-Object { $_.CommandLine }",
        ])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| {
            value
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn normalize_process_arg_path(path: &Path) -> String {
    normalize_process_text(&path.display().to_string())
}

fn normalize_process_text(value: &str) -> String {
    value
        .replace('"', "")
        .replace('/', "\\")
        .to_ascii_lowercase()
}

fn create_debug_target(endpoint: &HttpEndpoint, target_url: &str) -> Option<Value> {
    for method in ["PUT", "GET"] {
        let response = devtools_request(endpoint, method, &format!("/json/new?{target_url}")).ok();
        if let Some(response) = response
            && let Ok(value) = serde_json::from_str(&response)
        {
            return Some(value);
        }
    }
    None
}

fn target_id_from_value(value: &Value) -> Option<String> {
    value
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpEndpoint {
    host: String,
    port: u16,
}

fn parse_http_endpoint(raw: &str) -> Option<HttpEndpoint> {
    let without_scheme = raw.strip_prefix("http://")?;
    let authority = without_scheme.split('/').next()?.trim();
    let (host, port) = authority.rsplit_once(':')?;
    Some(HttpEndpoint {
        host: host.trim_matches(['[', ']']).to_owned(),
        port: port.parse().ok()?,
    })
}

fn devtools_request(endpoint: &HttpEndpoint, method: &str, path: &str) -> Result<String> {
    let mut stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port))
        .with_context(|| format!("connect to Chrome debug endpoint {}", endpoint.port))?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(600)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(600)));
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
        endpoint.host, endpoint.port
    )
    .context("write Chrome debug endpoint request")?;
    let response =
        read_http_response(&mut stream).context("read Chrome debug endpoint response")?;
    let (head, body) = response.split_once("\r\n\r\n").unwrap_or(("", &response));
    if !head.contains(" 200 ") {
        anyhow::bail!("Chrome debug endpoint returned non-200 response");
    }
    Ok(body.to_owned())
}

fn read_http_response(stream: &mut TcpStream) -> Result<String> {
    let mut response = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(count) => {
                response.extend_from_slice(&chunk[..count]);
                if response_body_complete(&response) {
                    break;
                }
            }
            Err(error)
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)
                    && !response.is_empty() =>
            {
                break;
            }
            Err(error) => return Err(error).context("read Chrome debug endpoint response"),
        }
    }
    String::from_utf8(response).context("decode Chrome debug endpoint response")
}

fn response_body_complete(response: &[u8]) -> bool {
    let Some(header_end) = response.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let head = String::from_utf8_lossy(&response[..header_end]);
    let Some(content_length) = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    }) else {
        return false;
    };
    response.len() >= header_end + 4 + content_length
}

fn same_target_url(current_url: &str, target_url: &str) -> bool {
    let Ok(current) = Url::parse(current_url.trim()) else {
        return current_url.trim().trim_end_matches('/') == target_url.trim().trim_end_matches('/');
    };
    let Ok(target) = Url::parse(target_url.trim()) else {
        return current_url.trim().trim_end_matches('/') == target_url.trim().trim_end_matches('/');
    };
    if current.scheme() != target.scheme() || current.host_str() != target.host_str() {
        return false;
    }
    match conversation_id(&target) {
        Some(target_conversation) => conversation_id(&current) == Some(target_conversation),
        None => workspace_path(&current) == workspace_path(&target),
    }
}

fn conversation_id(url: &Url) -> Option<&str> {
    let segments = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())?;
    if segments.len() >= 2 && segments[segments.len() - 2].eq_ignore_ascii_case("c") {
        segments.last().copied().filter(|value| !value.is_empty())
    } else {
        None
    }
}

fn workspace_path(url: &Url) -> String {
    let mut segments = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .unwrap_or_default();
    if segments.len() >= 2 && segments[segments.len() - 2].eq_ignore_ascii_case("c") {
        segments.truncate(segments.len() - 2);
    }
    if segments.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", segments.join("/"))
    }
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
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
    fn browser_start_debug_serialized_output_includes_chrome_attach_profile_fields() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("config")).unwrap();
        fs::write(
            dir.path().join("config").join("config.toml"),
            "version = 2\nbrowser_channel = \"chrome\"\nbrowser_debug_endpoint = \"http://127.0.0.1:1\"\n",
        )
        .unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();
        let browser_path = create_fake_browser_launcher(dir.path(), "chrome");

        let value = browser_start_debug(
            None,
            &config,
            Some("https://chatgpt.com/"),
            Some(&browser_path),
        )
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
        assert_eq!(value.get("status").and_then(Value::as_str), Some("started"));
        assert!(value.get("pid").is_none());
        assert!(value.get("launched_pid").and_then(Value::as_u64).is_some());
        assert_eq!(
            value
                .get("requires_clean_browser_restart_for_fresh_debug_endpoint")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value
                .get("target_url_opened_or_focused")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value.get("launched_new_process").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value
                .get("reused_existing_debug_endpoint")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert!(value.get("close_existing_browser_windows_first").is_none());
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
    fn start_debug_reuses_existing_debug_target_without_launching_duplicate() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("config")).unwrap();
        let (endpoint, requests) =
            start_fake_devtools_server(Some("https://chatgpt.com/c/existing"));
        fs::write(
            dir.path().join("config").join("config.toml"),
            format!("version = 2\nbrowser_channel = \"chrome\"\nbrowser_debug_endpoint = \"{endpoint}\"\n"),
        )
        .unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();
        let browser_path = create_fake_browser_launcher(dir.path(), "chrome");

        let value = browser_start_debug(
            None,
            &config,
            Some("https://chatgpt.com/"),
            Some(&browser_path),
        )
        .unwrap();
        thread::sleep(Duration::from_millis(50));
        let request_lines = requests.try_iter().collect::<Vec<_>>();

        assert_eq!(
            value.get("status").and_then(Value::as_str),
            Some("already_running")
        );
        assert!(value.get("pid").is_none());
        assert_eq!(value.get("launched_pid"), Some(&Value::Null));
        assert_eq!(
            value.get("action").and_then(Value::as_str),
            Some("reused_existing_debug_browser")
        );
        assert_eq!(
            value.get("launched_new_process").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(value.get("launch_args"), Some(&Value::Null));
        assert_eq!(
            value
                .get("reused_existing_debug_endpoint")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value
                .get("target_url_opened_or_focused")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value
                .get("target_tab_found_before_open")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value.get("target_tab_found").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value.get("target_tab_id").and_then(Value::as_str),
            Some("tab-1")
        );
        assert!(request_lines.iter().any(|line| line.contains("/json/list")));
        assert!(
            request_lines
                .iter()
                .any(|line| line.contains("/json/activate/tab-1"))
        );
        assert!(
            !request_lines
                .iter()
                .any(|line| line.contains("/json/reload"))
        );
        assert!(!request_lines.iter().any(|line| line.contains("/json/new")));
    }

    #[test]
    fn start_debug_opens_one_target_in_existing_debug_browser_when_missing() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("config")).unwrap();
        let (endpoint, requests) = start_fake_devtools_server(None);
        fs::write(
            dir.path().join("config").join("config.toml"),
            format!("version = 2\nbrowser_channel = \"chrome\"\nbrowser_debug_endpoint = \"{endpoint}\"\n"),
        )
        .unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();
        let browser_path = create_fake_browser_launcher(dir.path(), "chrome");

        let value = browser_start_debug(
            None,
            &config,
            Some("https://chatgpt.com/"),
            Some(&browser_path),
        )
        .unwrap();
        thread::sleep(Duration::from_millis(50));
        let request_lines = requests.try_iter().collect::<Vec<_>>();
        let new_requests = request_lines
            .iter()
            .filter(|line| line.contains("/json/new"))
            .count();

        assert_eq!(
            value.get("status").and_then(Value::as_str),
            Some("already_running")
        );
        assert_eq!(
            value.get("action").and_then(Value::as_str),
            Some("opened_target_in_existing_debug_browser")
        );
        assert_eq!(
            value.get("launched_new_process").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(value.get("launch_args"), Some(&Value::Null));
        assert_eq!(
            value
                .get("target_tab_found_before_open")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            value.get("target_tab_found").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value
                .get("target_url_opened_or_focused")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value.get("target_tab_id").and_then(Value::as_str),
            Some("created-tab")
        );
        assert_eq!(new_requests, 1);
    }

    #[test]
    fn start_debug_attached_ready_output_preserves_daemon_target_without_focus() {
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

        assert_eq!(
            value.get("status").and_then(Value::as_str),
            Some("already_running")
        );
        assert_eq!(
            value.get("action").and_then(Value::as_str),
            Some("reused_existing_debug_browser")
        );
        assert_eq!(
            value.get("target_tab_id").and_then(Value::as_str),
            Some("stable-attached-tab")
        );
        assert_eq!(
            value
                .get("target_url_opened_or_focused")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            value.get("target_tab_focused").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(value.get("launch_args"), Some(&Value::Null));
        assert_eq!(value.get("launched_pid"), Some(&Value::Null));
        assert_eq!(
            value.get("authenticated").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value.get("extension_connected").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn chrome_attach_profile_process_detection_matches_user_data_dir() {
        let profile_dir =
            Path::new("C:\\Users\\rdime\\AppData\\Local\\Tablinq\\chrome-attach-profile");
        let command_lines = [
            r#""C:\Program Files\Google\Chrome\Application\chrome.exe" --type=renderer"#,
            r#""C:\Program Files\Google\Chrome\Application\chrome.exe" --remote-debugging-port=9222 --user-data-dir="C:\Users\rdime\AppData\Local\Tablinq\chrome-attach-profile" --no-first-run https://chatgpt.com/"#,
        ];

        assert!(command_lines_include_attach_profile(
            command_lines,
            profile_dir
        ));
        assert!(!command_lines_include_attach_profile(
            [
                r#""C:\Program Files\Google\Chrome\Application\chrome.exe" --remote-debugging-port=9222 --user-data-dir="C:\Users\rdime\AppData\Local\Other""#
            ],
            profile_dir
        ));
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

    fn start_fake_devtools_server(target_url: Option<&str>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let target_url = target_url.map(ToOwned::to_owned);
        thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while std::time::Instant::now() < deadline {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let mut buffer = [0_u8; 2048];
                let count = stream.read(&mut buffer).unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..count]).to_string();
                let request_line = request.lines().next().unwrap_or("").to_owned();
                let _ = tx.send(request_line.clone());
                let body = if request_line.contains("/json/list") {
                    target_url
                        .as_ref()
                        .map(|url| format!(r#"[{{"id":"tab-1","type":"page","url":"{url}"}}]"#))
                        .unwrap_or_else(|| "[]".to_owned())
                } else if request_line.contains("/json/new") {
                    r#"{"id":"created-tab","type":"page","url":"https://chatgpt.com/"}"#.to_owned()
                } else {
                    "{}".to_owned()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        (endpoint, rx)
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
