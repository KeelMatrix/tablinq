use anyhow::Result;
use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

pub(super) fn attach_profile_process_running(attach_profile_dir: Option<&Path>) -> bool {
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

pub(super) fn command_lines_include_attach_profile<I>(
    command_lines: I,
    attach_profile_dir: &Path,
) -> bool
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

pub(super) fn chrome_attach_profile_dir_for_start_debug(
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

pub(super) fn resolve_browser_executable(
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

pub(super) fn ordered_browser_candidates<'a>(
    channel: &str,
    candidates: &'a [&'a str],
) -> Vec<&'a str> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;
    use tempfile::tempdir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
    fn ordered_browser_candidates_prefer_configured_channel_then_fallback() {
        let candidates = [
            "other-browser",
            "microsoft-edge",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "msedge.exe",
            "chrome.exe",
        ];

        assert_eq!(
            ordered_browser_candidates("chrome", &candidates),
            vec![
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "chrome.exe",
                "microsoft-edge",
                "msedge.exe",
                "other-browser"
            ]
        );
        assert_eq!(
            ordered_browser_candidates("msedge", &candidates),
            vec![
                "microsoft-edge",
                "msedge.exe",
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "chrome.exe",
                "other-browser"
            ]
        );
    }

    #[test]
    fn resolve_browser_executable_prefers_explicit_path_then_config_then_env() {
        let _guard = ENV_LOCK.lock().unwrap();
        let previous = env::var("TABLINQ_BROWSER_PATH").ok();
        unsafe {
            env::set_var("TABLINQ_BROWSER_PATH", "C:\\Env\\chrome.exe");
        }
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("config")).unwrap();
        fs::write(
            dir.path().join("config").join("config.toml"),
            "version = 2\nbrowser_channel = \"chrome\"\nbrowser_path = \"C:\\\\Config\\\\chrome.exe\"\n",
        )
        .unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();

        assert_eq!(
            resolve_browser_executable(&config, Some(Path::new("C:\\Cli\\chrome.exe"))).unwrap(),
            "C:\\Cli\\chrome.exe"
        );
        assert_eq!(
            resolve_browser_executable(&config, None).unwrap(),
            "C:\\Config\\chrome.exe"
        );

        fs::write(
            dir.path().join("config").join("config.toml"),
            "version = 2\nbrowser_channel = \"chrome\"\n",
        )
        .unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();
        assert_eq!(
            resolve_browser_executable(&config, None).unwrap(),
            "C:\\Env\\chrome.exe"
        );

        unsafe {
            if let Some(previous) = previous {
                env::set_var("TABLINQ_BROWSER_PATH", previous);
            } else {
                env::remove_var("TABLINQ_BROWSER_PATH");
            }
        }
    }
}
