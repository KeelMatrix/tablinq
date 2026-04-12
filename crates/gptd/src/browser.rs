use crate::config::AppConfig;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use shared_protocol::{
    AskCommand, AskResult, BrowserOpenRequest, BrowserStatusSnapshot, CapabilityReport, Effort,
    EventPayload, JobStatus, Mode, ModelClass, ResearchCommand, ResearchResult, StreamChunk,
    StreamChunkKind,
};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub struct BrowserManager {
    config: AppConfig,
    process: Option<AdapterProcess>,
    last_snapshot: BrowserStatusSnapshot,
}

#[derive(Debug, Clone)]
pub struct ResearchUpdate {
    pub status: JobStatus,
    pub message: Option<String>,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub current_url: Option<String>,
    pub chunk_kind: Option<StreamChunkKind>,
    pub chunk_text: Option<String>,
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

    pub fn ask<F>(
        &mut self,
        job_id: &str,
        command: &AskCommand,
        mut on_event: F,
    ) -> Result<AskResult>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        self.ensure_process()?;

        match self
            .process
            .as_mut()
            .expect("browser adapter process missing")
            .ask(job_id, command, &mut on_event)
        {
            Ok((snapshot, result)) => {
                self.last_snapshot = snapshot;
                Ok(result)
            }
            Err(err) => {
                let _ = self.health();
                Err(err)
            }
        }
    }

    pub fn research<F, G>(
        &mut self,
        job_id: &str,
        command: &ResearchCommand,
        mut on_update: F,
        mut should_cancel: G,
    ) -> Result<ResearchResult>
    where
        F: FnMut(ResearchUpdate) -> Result<()>,
        G: FnMut() -> bool,
    {
        self.ensure_process()?;

        match self
            .process
            .as_mut()
            .expect("browser adapter process missing")
            .research(job_id, command, &mut on_update, &mut should_cancel)
        {
            Ok((snapshot, result)) => {
                self.last_snapshot = snapshot;
                Ok(result)
            }
            Err(err) => {
                let _ = self.health();
                Err(err)
            }
        }
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

        let response: AdapterFrame =
            serde_json::from_str(line.trim_end()).context("decode adapter response")?;
        match response {
            AdapterFrame::Response { ok, state, error } => {
                if ok {
                    Ok(state)
                } else {
                    bail!(
                        "{}",
                        error.unwrap_or_else(|| {
                            "browser adapter returned an unknown error".to_owned()
                        })
                    );
                }
            }
            other => bail!("unexpected adapter frame for request: {other:?}"),
        }
    }

    fn ask<F>(
        &mut self,
        job_id: &str,
        command: &AskCommand,
        on_event: &mut F,
    ) -> Result<(BrowserStatusSnapshot, AskResult)>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let request = AdapterRequest::Ask {
            job_id: job_id.to_owned(),
            prompt: command.prompt.clone(),
            conversation_id: command.conversation_id.clone(),
            timeout_ms: Some(15_000),
            mode: Some(command.mode),
            model_class: command.model_class,
            effort: command.effort,
            files: command.files.clone(),
        };
        let raw = serde_json::to_string(&request).context("encode adapter ask request")?;
        self.stdin
            .write_all(raw.as_bytes())
            .context("write adapter ask request")?;
        self.stdin
            .write_all(b"\n")
            .context("write adapter newline")?;
        self.stdin.flush().context("flush adapter ask request")?;

        loop {
            let mut line = String::new();
            self.stdout
                .read_line(&mut line)
                .context("read adapter ask frame")?;
            if line.trim().is_empty() {
                bail!("browser adapter closed without an ask result");
            }

            let frame: AdapterFrame =
                serde_json::from_str(line.trim_end()).context("decode adapter ask frame")?;
            match frame {
                AdapterFrame::StreamChunk { chunk } => {
                    on_event(EventPayload::StreamChunk(StreamChunk {
                        job_id: chunk.job_id,
                        conversation_id: chunk.conversation_id,
                        message_id: chunk.message_id,
                        seq: chunk.seq,
                        kind: if chunk.kind == "done" {
                            shared_protocol::StreamChunkKind::Done
                        } else {
                            shared_protocol::StreamChunkKind::Delta
                        },
                        text: chunk.text,
                    }))?;
                }
                AdapterFrame::AskResult {
                    ok,
                    state,
                    result,
                    error,
                } => {
                    if ok {
                        return Ok((
                            state,
                            result.context("adapter ask finished without a result")?,
                        ));
                    }
                    bail!(
                        "{}",
                        error.unwrap_or_else(|| {
                            "browser adapter returned an unknown ask error".to_owned()
                        })
                    );
                }
                AdapterFrame::Response {
                    ok: false, error, ..
                } => {
                    bail!(
                        "{}",
                        error.unwrap_or_else(|| {
                            "browser adapter returned an unknown ask error".to_owned()
                        })
                    );
                }
                AdapterFrame::Response { .. } => {
                    bail!("unexpected adapter response while asking")
                }
                AdapterFrame::ResearchUpdate { .. } | AdapterFrame::ResearchResult { .. } => {
                    bail!("unexpected adapter research frame while asking")
                }
            }
        }
    }

    fn research<F, G>(
        &mut self,
        job_id: &str,
        command: &ResearchCommand,
        on_update: &mut F,
        should_cancel: &mut G,
    ) -> Result<(BrowserStatusSnapshot, ResearchResult)>
    where
        F: FnMut(ResearchUpdate) -> Result<()>,
        G: FnMut() -> bool,
    {
        let request = AdapterRequest::Research {
            job_id: job_id.to_owned(),
            query: command.query.clone(),
            files: command.files.clone(),
            output_dir: command.output_dir.clone(),
            timeout_ms: Some(30_000),
        };
        let raw = serde_json::to_string(&request).context("encode adapter research request")?;
        self.stdin
            .write_all(raw.as_bytes())
            .context("write adapter research request")?;
        self.stdin
            .write_all(b"\n")
            .context("write adapter newline")?;
        self.stdin
            .flush()
            .context("flush adapter research request")?;

        loop {
            let mut line = String::new();
            self.stdout
                .read_line(&mut line)
                .context("read adapter research frame")?;
            if line.trim().is_empty() {
                bail!("browser adapter closed without a research result");
            }

            let frame: AdapterFrame =
                serde_json::from_str(line.trim_end()).context("decode adapter research frame")?;
            match frame {
                AdapterFrame::ResearchUpdate { update } => {
                    on_update(ResearchUpdate {
                        status: update.status,
                        message: update.message,
                        conversation_id: update.conversation_id,
                        message_id: update.message_id,
                        current_url: update.current_url,
                        chunk_kind: update.chunk_kind,
                        chunk_text: update.chunk_text,
                    })?;
                    if should_cancel() {
                        let _ = self.child.kill();
                        let _ = self.child.wait();
                        bail!("research cancelled");
                    }
                }
                AdapterFrame::ResearchResult {
                    ok,
                    state,
                    result,
                    error,
                } => {
                    if ok {
                        return Ok((
                            state,
                            result.context("adapter research finished without a result")?,
                        ));
                    }
                    bail!(
                        "{}",
                        error.unwrap_or_else(|| {
                            "browser adapter returned an unknown research error".to_owned()
                        })
                    );
                }
                AdapterFrame::Response {
                    ok: false, error, ..
                } => {
                    bail!(
                        "{}",
                        error.unwrap_or_else(|| {
                            "browser adapter returned an unknown research error".to_owned()
                        })
                    );
                }
                AdapterFrame::Response { .. } => {
                    bail!("unexpected adapter response while running research")
                }
                AdapterFrame::StreamChunk { .. } | AdapterFrame::AskResult { .. } => {
                    bail!("unexpected adapter frame while running research")
                }
            }
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
    Ask {
        job_id: String,
        prompt: String,
        conversation_id: Option<String>,
        timeout_ms: Option<u64>,
        mode: Option<Mode>,
        model_class: Option<ModelClass>,
        effort: Option<Effort>,
        files: Vec<String>,
    },
    Research {
        job_id: String,
        query: String,
        files: Vec<String>,
        output_dir: Option<String>,
        timeout_ms: Option<u64>,
    },
    Close,
    Shutdown,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AdapterFrame {
    Response {
        ok: bool,
        state: BrowserStatusSnapshot,
        error: Option<String>,
    },
    StreamChunk {
        chunk: AdapterStreamChunk,
    },
    AskResult {
        ok: bool,
        state: BrowserStatusSnapshot,
        result: Option<AskResult>,
        error: Option<String>,
    },
    ResearchUpdate {
        update: AdapterResearchUpdate,
    },
    ResearchResult {
        ok: bool,
        state: BrowserStatusSnapshot,
        result: Option<ResearchResult>,
        error: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdapterStreamChunk {
    job_id: String,
    conversation_id: Option<String>,
    message_id: Option<String>,
    seq: u64,
    kind: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdapterResearchUpdate {
    status: JobStatus,
    message: Option<String>,
    conversation_id: Option<String>,
    message_id: Option<String>,
    current_url: Option<String>,
    chunk_kind: Option<StreamChunkKind>,
    chunk_text: Option<String>,
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
