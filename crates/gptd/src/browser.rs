use crate::config::AppConfig;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use shared_protocol::{BrowserOpenRequest, BrowserStatusSnapshot, CapabilityReport};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub struct BrowserManager {
    config: AppConfig,
    process: Option<AdapterProcess>,
    last_snapshot: BrowserStatusSnapshot,
}

impl BrowserManager {
    pub fn new(config: &AppConfig) -> Self {
        Self {
            config: config.clone(),
            process: None,
            last_snapshot: idle_snapshot(config, None, None),
        }
    }

    pub fn open(&mut self, request: BrowserOpenRequest) -> Result<BrowserStatusSnapshot> {
        self.ensure_process()?;

        let target_url = request
            .target_url
            .unwrap_or_else(|| self.config.browser_target_url().to_owned());
        let browser_path = request
            .browser_path
            .or_else(|| self.config.browser_path().map(ToOwned::to_owned));
        let headless = request.headless.unwrap_or(false);

        let request_browser_path = browser_path.clone();
        match self
            .process
            .as_mut()
            .expect("browser adapter process missing")
            .request(AdapterRequest::Open {
                target_url,
                browser_path,
                headless,
            }) {
            Ok(snapshot) => {
                self.last_snapshot = snapshot.clone();
                Ok(snapshot)
            }
            Err(err) => {
                self.teardown_process();
                self.last_snapshot = idle_snapshot(
                    &self.config,
                    Some(self.current_target()),
                    request_browser_path,
                );
                Err(err)
            }
        }
    }

    pub fn close(&mut self) -> Result<BrowserStatusSnapshot> {
        if let Some(process) = self.process.as_mut() {
            let _ = process.request(AdapterRequest::Close);
            let _ = process.request(AdapterRequest::Shutdown);
        }
        self.teardown_process();
        let snapshot = idle_snapshot(
            &self.config,
            Some(self.last_snapshot.target_url.clone()),
            self.last_snapshot.browser_path.clone(),
        );
        self.last_snapshot = snapshot.clone();
        Ok(snapshot)
    }

    pub fn profile_info(&mut self) -> Result<BrowserStatusSnapshot> {
        self.health()
    }

    pub fn capabilities(&mut self) -> Result<Option<CapabilityReport>> {
        Ok(self.health()?.capability_report)
    }

    pub fn health(&mut self) -> Result<BrowserStatusSnapshot> {
        if self.process.is_none() {
            return Ok(self.last_snapshot.clone());
        }

        match self
            .process
            .as_mut()
            .expect("browser adapter process missing")
            .request(AdapterRequest::Health)
        {
            Ok(snapshot) => {
                self.last_snapshot = snapshot.clone();
                Ok(snapshot)
            }
            Err(_) => {
                self.teardown_process();
                let snapshot = idle_snapshot(
                    &self.config,
                    Some(self.last_snapshot.target_url.clone()),
                    self.last_snapshot.browser_path.clone(),
                );
                self.last_snapshot = snapshot.clone();
                Ok(snapshot)
            }
        }
    }

    pub fn reset_profile(&mut self) -> Result<BrowserStatusSnapshot> {
        let _ = self.close();
        if self.config.paths.browser_profile_dir.exists() {
            fs::remove_dir_all(&self.config.paths.browser_profile_dir).with_context(|| {
                format!("remove {}", self.config.paths.browser_profile_dir.display())
            })?;
        }
        fs::create_dir_all(&self.config.paths.browser_profile_dir).with_context(|| {
            format!("create {}", self.config.paths.browser_profile_dir.display())
        })?;

        let snapshot = idle_snapshot(&self.config, None, None);
        self.last_snapshot = snapshot.clone();
        Ok(snapshot)
    }

    fn ensure_process(&mut self) -> Result<()> {
        if self.process.is_some() {
            return Ok(());
        }

        self.process = Some(AdapterProcess::spawn(&self.config)?);
        Ok(())
    }

    fn teardown_process(&mut self) {
        if let Some(mut process) = self.process.take() {
            let _ = process.child.kill();
            let _ = process.child.wait();
        }
    }

    fn current_target(&self) -> String {
        self.last_snapshot.target_url.clone()
    }
}

impl Drop for BrowserManager {
    fn drop(&mut self) {
        self.teardown_process();
    }
}

struct AdapterProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl AdapterProcess {
    fn spawn(config: &AppConfig) -> Result<Self> {
        let entry = resolve_adapter_entry();
        let mut command = Command::new(if cfg!(windows) { "npx.cmd" } else { "npx" });
        command
            .arg("tsx")
            .arg(entry)
            .arg("--profile-dir")
            .arg(&config.paths.browser_profile_dir)
            .arg("--downloads-dir")
            .arg(&config.paths.downloads_dir)
            .arg("--default-target-url")
            .arg(config.browser_target_url())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        if let Some(browser_path) = config.browser_path() {
            command.arg("--default-browser-path").arg(browser_path);
        }

        let mut child = command.spawn().context("spawn browser adapter")?;
        let stdin = child.stdin.take().context("adapter stdin unavailable")?;
        let stdout = child.stdout.take().context("adapter stdout unavailable")?;

        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    fn request(&mut self, request: AdapterRequest) -> Result<BrowserStatusSnapshot> {
        let raw = serde_json::to_string(&request).context("encode adapter request")?;
        self.stdin
            .write_all(raw.as_bytes())
            .context("write adapter request")?;
        self.stdin
            .write_all(b"\n")
            .context("write adapter newline")?;
        self.stdin.flush().context("flush adapter request")?;

        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .context("read adapter response")?;
        if line.trim().is_empty() {
            bail!("browser adapter closed without a response");
        }

        let response: AdapterResponse =
            serde_json::from_str(line.trim_end()).context("decode adapter response")?;
        if response.ok {
            Ok(response.state)
        } else {
            bail!(
                "{}",
                response
                    .error
                    .unwrap_or_else(|| "browser adapter returned an unknown error".to_owned())
            );
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AdapterRequest {
    Open {
        target_url: String,
        browser_path: Option<String>,
        headless: bool,
    },
    Health,
    Close,
    Shutdown,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdapterResponse {
    ok: bool,
    state: BrowserStatusSnapshot,
    error: Option<String>,
}

fn idle_snapshot(
    config: &AppConfig,
    target_url: Option<String>,
    browser_path: Option<String>,
) -> BrowserStatusSnapshot {
    BrowserStatusSnapshot {
        running: false,
        browser_kind: if cfg!(windows) {
            "msedge".to_owned()
        } else {
            "chromium".to_owned()
        },
        profile_dir: config.paths.browser_profile_dir.display().to_string(),
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
    }
}

fn resolve_adapter_entry() -> PathBuf {
    if let Ok(value) = std::env::var("TABLINQ_BROWSER_ADAPTER_ENTRY") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("packages")
        .join("browser-adapter")
        .join("src")
        .join("service.ts")
}
