use crate::browser::detection::{
    attach_profile_process_running, chrome_attach_profile_dir_for_start_debug,
    resolve_browser_executable,
};
use crate::browser::devtools::{
    focus_existing_debug_target_before_launch, wait_for_existing_debug_target,
};
use crate::browser::output::{
    AttachedDaemonBrowser, attached_daemon_browser_output, existing_debug_browser_starting_output,
    existing_debug_target_output, started_debug_browser_output,
};
use crate::browser::urls::same_target_url;
use crate::daemon::send_daemon_request;
use anyhow::{Context, Result};
use serde_json::Value;
use shared_protocol::DaemonRequest;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct BrowserStartDebugPlan {
    pub(super) executable: String,
    pub(super) launch_args: Vec<String>,
    pub(super) target_url: String,
    pub(super) attach_profile_dir: Option<PathBuf>,
}

pub(super) fn browser_start_debug(
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
        return Ok(existing_debug_browser_starting_output(config, &plan));
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

    Ok(started_debug_browser_output(config, &plan, child.id()))
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

pub(super) fn build_browser_start_debug_plan(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::output::browser_start_debug_note;
    use serde_json::Value;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

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
        assert_eq!(plan.target_url, "https://chatgpt.com/");
        assert_eq!(
            plan.launch_args,
            vec![
                "--remote-debugging-port=9222".to_owned(),
                "--remote-debugging-address=127.0.0.1".to_owned(),
                format!(
                    "--user-data-dir={}",
                    config.paths.chrome_attach_profile_dir.display()
                ),
                "--no-first-run".to_owned(),
                "--no-default-browser-check".to_owned(),
                "https://chatgpt.com/".to_owned()
            ]
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
        assert_eq!(plan.target_url, "https://chatgpt.com/");
        assert_eq!(plan.attach_profile_dir, None);
        assert!(
            !plan
                .launch_args
                .iter()
                .any(|arg| arg.contains("--user-data-dir"))
        );
    }

    #[test]
    fn start_debug_plan_uses_configured_target_url_when_cli_target_is_blank() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("config")).unwrap();
        fs::write(
            dir.path().join("config").join("config.toml"),
            "version = 2\nbrowser_target_url = \"https://chatgpt.com/g/g-workspace\"\n",
        )
        .unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();

        let plan = build_browser_start_debug_plan(
            &config,
            Some("  "),
            Some(Path::new("C:\\Browsers\\Edge\\msedge.exe")),
        )
        .unwrap();

        assert_eq!(plan.target_url, "https://chatgpt.com/g/g-workspace");
        assert_eq!(
            plan.launch_args.last().map(String::as_str),
            Some("https://chatgpt.com/g/g-workspace")
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
}
