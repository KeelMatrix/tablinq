use crate::config::AppConfig;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use shared_protocol::{
    AskCommand, AskResult, BrowserOpenRequest, BrowserStatusSnapshot, CapabilityReport, Effort,
    EventPayload, JobStatus, Mode, ModelClass, ResearchCommand, ResearchResult,
    SelectorPackSummary, StrategyAttempt, StrategyTelemetryEntry, StreamChunk, StreamChunkKind,
};
use std::cell::{Cell, RefCell};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const ADAPTER_CONTROL_TIMEOUT: Duration = Duration::from_secs(30);

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

#[derive(Debug, Clone)]
pub struct RequestLifecycleUpdate {
    pub status: JobStatus,
    pub message: String,
    pub conversation_id: Option<String>,
    pub message_id: Option<String>,
    pub current_url: Option<String>,
    pub streaming_safe: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BrowserDiagnosticsSnapshot {
    pub current_url: Option<String>,
    pub capability_matrix: shared_protocol::CapabilityMatrix,
    pub strategy_attempts: Vec<StrategyAttempt>,
    pub dom_snapshot: String,
    pub aria_snapshot: String,
    pub screenshot_path: Option<String>,
    pub selector_pack: Option<SelectorPackSummary>,
    pub strategy_telemetry: Vec<StrategyTelemetryEntry>,
    pub browser_detected: bool,
    pub detected_browser_path: Option<String>,
    pub profile_accessible: bool,
    pub profile_access_error: Option<String>,
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

    pub fn logout(&mut self) -> Result<BrowserStatusSnapshot> {
        let target_url = self.current_target();
        let browser_path = self.last_snapshot.browser_path.clone();
        self.reset_profile()?;
        self.open(BrowserOpenRequest {
            target_url: Some(target_url),
            browser_path,
            headless: Some(false),
        })
    }

    pub fn profile_info(&mut self) -> Result<BrowserStatusSnapshot> {
        self.health()
    }

    pub fn capabilities(&mut self) -> Result<Option<CapabilityReport>> {
        Ok(self.health()?.capability_report)
    }

    pub fn diagnostics(&mut self, bundle_dir: Option<&Path>) -> Result<BrowserDiagnosticsSnapshot> {
        self.ensure_process()?;
        self.process
            .as_mut()
            .expect("browser adapter process missing")
            .diagnostics(bundle_dir)
    }

    pub fn ask<F>(
        &mut self,
        job_id: &str,
        command: &AskCommand,
        mut on_event: F,
        mut on_state: impl FnMut(RequestLifecycleUpdate) -> Result<()>,
        mut should_cancel: impl FnMut() -> bool,
    ) -> Result<AskResult>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        self.ensure_process()?;
        let emitted_text = RefCell::new(String::new());
        let next_seq = Cell::new(0u64);
        let mut forwarding = |event: EventPayload| {
            if let EventPayload::StreamChunk(chunk) = &event
                && chunk.kind == StreamChunkKind::Delta
                && !chunk.text.is_empty()
            {
                emitted_text.borrow_mut().push_str(&chunk.text);
                next_seq.set(chunk.seq + 1);
            }
            on_event(event)
        };

        match self
            .process
            .as_mut()
            .expect("browser adapter process missing")
            .ask(
                job_id,
                command,
                self.config.ask_timeout_ms(),
                &mut forwarding,
                &mut on_state,
                &mut should_cancel,
            ) {
            Ok((snapshot, result)) => {
                self.last_snapshot = snapshot;
                Ok(result)
            }
            Err(err) => {
                if is_recoverable_adapter_error(&err.to_string()) {
                    on_state(RequestLifecycleUpdate {
                        status: JobStatus::Interrupted,
                        message: format!("ask interrupted: {err:#}"),
                        conversation_id: command.conversation_id.clone(),
                        message_id: None,
                        current_url: self.last_snapshot.current_url.clone(),
                        streaming_safe: false,
                    })?;
                    self.relaunch_for_recovery()?;
                    let (snapshot, result) = self
                        .process
                        .as_mut()
                        .expect("browser adapter process missing")
                        .recover_ask(
                            job_id,
                            command,
                            self.config.ask_timeout_ms(),
                            emitted_text.borrow().as_str(),
                            next_seq.get(),
                            &mut forwarding,
                            &mut on_state,
                            &mut should_cancel,
                        )?;
                    self.last_snapshot = snapshot;
                    return Ok(result);
                }
                if should_reset_after_failure(&err.to_string()) {
                    self.teardown_process();
                    self.last_snapshot = idle_snapshot(
                        &self.config,
                        Some(self.current_target()),
                        self.last_snapshot.browser_path.clone(),
                    );
                }
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
        mut on_state: impl FnMut(RequestLifecycleUpdate) -> Result<()>,
        mut should_cancel: G,
    ) -> Result<ResearchResult>
    where
        F: FnMut(ResearchUpdate) -> Result<()>,
        G: FnMut() -> bool,
    {
        self.ensure_process()?;
        let emitted_text = RefCell::new(String::new());
        let next_seq = Cell::new(0u64);
        let mut forwarding = |update: ResearchUpdate| {
            if let Some(chunk_text) = &update.chunk_text
                && update.chunk_kind.unwrap_or(StreamChunkKind::Delta) == StreamChunkKind::Delta
                && !chunk_text.is_empty()
            {
                emitted_text.borrow_mut().push_str(chunk_text);
                next_seq.set(next_seq.get() + 1);
            }
            on_update(update)
        };

        match self
            .process
            .as_mut()
            .expect("browser adapter process missing")
            .research(
                job_id,
                command,
                self.config.research_timeout_ms(),
                &mut forwarding,
                &mut on_state,
                &mut should_cancel,
            ) {
            Ok((snapshot, result)) => {
                self.last_snapshot = snapshot;
                Ok(result)
            }
            Err(err) => {
                if is_recoverable_adapter_error(&err.to_string()) {
                    on_state(RequestLifecycleUpdate {
                        status: JobStatus::Interrupted,
                        message: format!("research interrupted: {err:#}"),
                        conversation_id: command.conversation_id.clone(),
                        message_id: None,
                        current_url: self.last_snapshot.current_url.clone(),
                        streaming_safe: false,
                    })?;
                    self.relaunch_for_recovery()?;
                    let (snapshot, result) = self
                        .process
                        .as_mut()
                        .expect("browser adapter process missing")
                        .recover_research(
                            job_id,
                            command,
                            self.config.research_timeout_ms(),
                            emitted_text.borrow().as_str(),
                            next_seq.get(),
                            &mut forwarding,
                            &mut on_state,
                            &mut should_cancel,
                        )?;
                    self.last_snapshot = snapshot;
                    return Ok(result);
                }
                if should_reset_after_failure(&err.to_string()) {
                    self.teardown_process();
                    self.last_snapshot = idle_snapshot(
                        &self.config,
                        Some(self.current_target()),
                        self.last_snapshot.browser_path.clone(),
                    );
                }
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
            process.terminate();
        }
    }

    fn relaunch_for_recovery(&mut self) -> Result<()> {
        self.teardown_process();
        self.ensure_process()?;
        Ok(())
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
    frames: Receiver<Result<AdapterFrame>>,
    reader_thread: Option<JoinHandle<()>>,
}

struct AdapterRuntime {
    entry: PathBuf,
    asset_root: PathBuf,
    launcher: AdapterLauncher,
}

enum AdapterLauncher {
    Node,
    Tsx(PathBuf),
}

impl AdapterProcess {
    fn spawn(config: &AppConfig) -> Result<Self> {
        let runtime = resolve_adapter_runtime()?;
        let mut command = match &runtime.launcher {
            AdapterLauncher::Node => Command::new(node_command()),
            AdapterLauncher::Tsx(path) => Command::new(path),
        };
        command.arg(&runtime.entry);
        command
            .arg("--profile-dir")
            .arg(&config.paths.browser_profile_dir)
            .arg("--downloads-dir")
            .arg(&config.paths.downloads_dir)
            .arg("--default-target-url")
            .arg(config.browser_target_url())
            .env("TABLINQ_ASSET_ROOT", &runtime.asset_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        if let Some(browser_path) = config.browser_path() {
            command.arg("--default-browser-path").arg(browser_path);
        }

        let mut child = command.spawn().context("spawn browser adapter")?;
        let stdin = child.stdin.take().context("adapter stdin unavailable")?;
        let stdout = child.stdout.take().context("adapter stdout unavailable")?;
        let (tx, rx) = mpsc::channel();
        let reader_thread = Some(spawn_adapter_reader(stdout, tx));

        Ok(Self {
            child,
            stdin,
            frames: rx,
            reader_thread,
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

        let response = self.next_frame_blocking("response")?;
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

    fn diagnostics(&mut self, bundle_dir: Option<&Path>) -> Result<BrowserDiagnosticsSnapshot> {
        let request = AdapterRequest::Diagnostics {
            bundle_dir: bundle_dir.map(|path| path.display().to_string()),
        };
        let raw = serde_json::to_string(&request).context("encode adapter diagnostics request")?;
        self.stdin
            .write_all(raw.as_bytes())
            .context("write adapter diagnostics request")?;
        self.stdin
            .write_all(b"\n")
            .context("write adapter newline")?;
        self.stdin
            .flush()
            .context("flush adapter diagnostics request")?;

        let response = self.next_frame_blocking("diagnostics response")?;
        match response {
            AdapterFrame::Diagnostics {
                ok,
                state: _state,
                diagnostics,
                error,
            } => {
                if ok {
                    diagnostics.context("adapter diagnostics finished without a result")
                } else {
                    bail!(
                        "{}",
                        error.unwrap_or_else(|| {
                            "browser adapter returned an unknown diagnostics error".to_owned()
                        })
                    );
                }
            }
            other => bail!("unexpected adapter frame for diagnostics: {other:?}"),
        }
    }

    fn ask<F>(
        &mut self,
        job_id: &str,
        command: &AskCommand,
        timeout_ms: u64,
        on_event: &mut F,
        on_state: &mut impl FnMut(RequestLifecycleUpdate) -> Result<()>,
        should_cancel: &mut impl FnMut() -> bool,
    ) -> Result<(BrowserStatusSnapshot, AskResult)>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        self.ask_impl(
            AdapterRequest::Ask {
                job_id: job_id.to_owned(),
                prompt: command.prompt.clone(),
                conversation_id: command.conversation_id.clone(),
                timeout_ms: Some(timeout_ms),
                mode: Some(command.mode),
                model_class: command.model_class,
                effort: command.effort,
                files: command.files.clone(),
                emitted_text: None,
                next_seq: None,
            },
            "ask",
            on_event,
            on_state,
            should_cancel,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn recover_ask<F>(
        &mut self,
        job_id: &str,
        command: &AskCommand,
        timeout_ms: u64,
        emitted_text: &str,
        next_seq: u64,
        on_event: &mut F,
        on_state: &mut impl FnMut(RequestLifecycleUpdate) -> Result<()>,
        should_cancel: &mut impl FnMut() -> bool,
    ) -> Result<(BrowserStatusSnapshot, AskResult)>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        self.ask_impl(
            AdapterRequest::RecoverAsk {
                job_id: job_id.to_owned(),
                prompt: command.prompt.clone(),
                conversation_id: command.conversation_id.clone(),
                timeout_ms: Some(timeout_ms),
                mode: Some(command.mode),
                model_class: command.model_class,
                effort: command.effort,
                files: command.files.clone(),
                emitted_text: Some(emitted_text.to_owned()),
                next_seq: Some(next_seq),
            },
            "ask recovery",
            on_event,
            on_state,
            should_cancel,
        )
    }

    fn ask_impl<F>(
        &mut self,
        request: AdapterRequest,
        context: &str,
        on_event: &mut F,
        on_state: &mut impl FnMut(RequestLifecycleUpdate) -> Result<()>,
        should_cancel: &mut impl FnMut() -> bool,
    ) -> Result<(BrowserStatusSnapshot, AskResult)>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let raw = serde_json::to_string(&request).context("encode adapter ask request")?;
        self.stdin
            .write_all(raw.as_bytes())
            .context("write adapter ask request")?;
        self.stdin
            .write_all(b"\n")
            .context("write adapter newline")?;
        self.stdin.flush().context("flush adapter ask request")?;

        loop {
            let frame = self.next_frame_interruptible(context, should_cancel)?;
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
                AdapterFrame::RequestState { update } => {
                    on_state(map_request_state_update(update))?;
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
                AdapterFrame::ResearchUpdate { .. }
                | AdapterFrame::ResearchResult { .. }
                | AdapterFrame::Diagnostics { .. } => {
                    bail!("unexpected adapter research frame while asking")
                }
            }
        }
    }

    fn research<F, G>(
        &mut self,
        job_id: &str,
        command: &ResearchCommand,
        timeout_ms: u64,
        on_update: &mut F,
        on_state: &mut impl FnMut(RequestLifecycleUpdate) -> Result<()>,
        should_cancel: &mut G,
    ) -> Result<(BrowserStatusSnapshot, ResearchResult)>
    where
        F: FnMut(ResearchUpdate) -> Result<()>,
        G: FnMut() -> bool,
    {
        self.research_impl(
            AdapterRequest::Research {
                job_id: job_id.to_owned(),
                query: command.query.clone(),
                conversation_id: command.conversation_id.clone(),
                files: command.files.clone(),
                output_dir: command.output_dir.clone(),
                timeout_ms: Some(timeout_ms),
                emitted_text: None,
                next_seq: None,
            },
            "research",
            on_update,
            on_state,
            should_cancel,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn recover_research<F, G>(
        &mut self,
        job_id: &str,
        command: &ResearchCommand,
        timeout_ms: u64,
        emitted_text: &str,
        next_seq: u64,
        on_update: &mut F,
        on_state: &mut impl FnMut(RequestLifecycleUpdate) -> Result<()>,
        should_cancel: &mut G,
    ) -> Result<(BrowserStatusSnapshot, ResearchResult)>
    where
        F: FnMut(ResearchUpdate) -> Result<()>,
        G: FnMut() -> bool,
    {
        self.research_impl(
            AdapterRequest::RecoverResearch {
                job_id: job_id.to_owned(),
                query: command.query.clone(),
                conversation_id: command.conversation_id.clone(),
                files: command.files.clone(),
                output_dir: command.output_dir.clone(),
                timeout_ms: Some(timeout_ms),
                emitted_text: Some(emitted_text.to_owned()),
                next_seq: Some(next_seq),
            },
            "research recovery",
            on_update,
            on_state,
            should_cancel,
        )
    }

    fn research_impl<F, G>(
        &mut self,
        request: AdapterRequest,
        context: &str,
        on_update: &mut F,
        on_state: &mut impl FnMut(RequestLifecycleUpdate) -> Result<()>,
        should_cancel: &mut G,
    ) -> Result<(BrowserStatusSnapshot, ResearchResult)>
    where
        F: FnMut(ResearchUpdate) -> Result<()>,
        G: FnMut() -> bool,
    {
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
            let frame = self.next_frame_interruptible(context, should_cancel)?;
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
                }
                AdapterFrame::RequestState { update } => {
                    on_state(map_request_state_update(update))?;
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
                AdapterFrame::StreamChunk { .. }
                | AdapterFrame::AskResult { .. }
                | AdapterFrame::Diagnostics { .. } => {
                    bail!("unexpected adapter frame while running research")
                }
            }
        }
    }

    fn next_frame_blocking(&mut self, context: &str) -> Result<AdapterFrame> {
        match self.frames.recv_timeout(ADAPTER_CONTROL_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                self.terminate();
                bail!(
                    "browser adapter timed out after {} ms while waiting for {context}",
                    ADAPTER_CONTROL_TIMEOUT.as_millis()
                );
            }
            Err(_) => bail!("browser adapter closed without a {context}"),
        }
    }

    fn next_frame_interruptible(
        &mut self,
        context: &str,
        should_cancel: &mut impl FnMut() -> bool,
    ) -> Result<AdapterFrame> {
        loop {
            if should_cancel() {
                self.terminate();
                bail!("{context} cancelled");
            }
            match self.frames.recv_timeout(Duration::from_millis(50)) {
                Ok(result) => return result,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    bail!("browser adapter closed without a {context} result")
                }
            }
        }
    }

    fn terminate(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader_thread) = self.reader_thread.take() {
            let _ = reader_thread.join();
        }
    }
}

pub fn browser_integration_prereqs_available() -> bool {
    if !command_available(node_command(), "--version") {
        return false;
    }

    let Ok(runtime) = resolve_adapter_runtime() else {
        return false;
    };

    has_playwright_core(&runtime.entry)
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
        emitted_text: Option<String>,
        next_seq: Option<u64>,
    },
    RecoverAsk {
        job_id: String,
        prompt: String,
        conversation_id: Option<String>,
        timeout_ms: Option<u64>,
        mode: Option<Mode>,
        model_class: Option<ModelClass>,
        effort: Option<Effort>,
        files: Vec<String>,
        emitted_text: Option<String>,
        next_seq: Option<u64>,
    },
    Research {
        job_id: String,
        query: String,
        conversation_id: Option<String>,
        files: Vec<String>,
        output_dir: Option<String>,
        timeout_ms: Option<u64>,
        emitted_text: Option<String>,
        next_seq: Option<u64>,
    },
    RecoverResearch {
        job_id: String,
        query: String,
        conversation_id: Option<String>,
        files: Vec<String>,
        output_dir: Option<String>,
        timeout_ms: Option<u64>,
        emitted_text: Option<String>,
        next_seq: Option<u64>,
    },
    Diagnostics {
        bundle_dir: Option<String>,
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
    RequestState {
        update: AdapterRequestStateUpdate,
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
    Diagnostics {
        ok: bool,
        state: BrowserStatusSnapshot,
        diagnostics: Option<BrowserDiagnosticsSnapshot>,
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdapterRequestStateUpdate {
    state: String,
    reason: String,
    at: String,
    conversation_id: Option<String>,
    message_id: Option<String>,
    current_url: Option<String>,
    streaming_safe: bool,
}

fn map_request_state_update(update: AdapterRequestStateUpdate) -> RequestLifecycleUpdate {
    let status = match update.state.as_str() {
        "recovered" => JobStatus::Recovered,
        _ => JobStatus::Interrupted,
    };
    RequestLifecycleUpdate {
        status,
        message: if update.at.is_empty() {
            update.reason
        } else {
            format!("{} ({})", update.reason, update.at)
        },
        conversation_id: update.conversation_id,
        message_id: update.message_id,
        current_url: update.current_url,
        streaming_safe: update.streaming_safe,
    }
}

fn is_recoverable_adapter_error(message: &str) -> bool {
    message.contains("Execution context was destroyed")
        || message.contains("Cannot find context")
        || message.contains("Frame has been detached")
        || message.contains("navigation")
        || message.contains("managed page is not open")
        || message.contains("Target page, context or browser has been closed")
        || message.contains("browser adapter closed without")
}

fn should_reset_after_failure(message: &str) -> bool {
    message.contains("cancelled")
        || message.contains("managed page is not open")
        || message.contains("Target page, context or browser has been closed")
        || message.contains("browser adapter closed without")
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

fn resolve_adapter_runtime() -> Result<AdapterRuntime> {
    if let Ok(value) = std::env::var("TABLINQ_BROWSER_ADAPTER_ENTRY") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let entry = PathBuf::from(trimmed);
            if entry.exists() {
                let asset_root = std::env::var("TABLINQ_ASSET_ROOT")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(PathBuf::from)
                    .or_else(|| infer_asset_root(&entry))
                    .unwrap_or_else(|| {
                        entry
                            .parent()
                            .unwrap_or_else(|| Path::new("."))
                            .to_path_buf()
                    });
                return build_adapter_runtime(entry, asset_root)
                    .context("resolve TABLINQ_BROWSER_ADAPTER_ENTRY");
            }
        }
    }

    let mut last_error = None;
    for (entry, asset_root) in adapter_runtime_candidates() {
        if entry.exists() {
            match build_adapter_runtime(entry, asset_root) {
                Ok(runtime) => return Ok(runtime),
                Err(err) => last_error = Some(err),
            }
        }
    }

    if let Some(err) = last_error {
        return Err(err).context("browser adapter entry could not be resolved");
    }

    bail!("browser adapter entry could not be resolved")
}

fn adapter_runtime_candidates() -> Vec<(PathBuf, PathBuf)> {
    let mut candidates = Vec::new();

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(bin_dir) = current_exe.parent()
        && let Some(install_root) = bin_dir.parent()
    {
        candidates.push((
            install_root
                .join("browser-adapter")
                .join("src")
                .join("service.js"),
            install_root.to_path_buf(),
        ));
        candidates.push((
            install_root
                .join("packages")
                .join("browser-adapter")
                .join("dist")
                .join("src")
                .join("service.js"),
            install_root.to_path_buf(),
        ));
    }

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    candidates.push((
        repo_root
            .join("packages")
            .join("browser-adapter")
            .join("dist")
            .join("src")
            .join("service.js"),
        repo_root.clone(),
    ));
    candidates.push((
        repo_root
            .join("packages")
            .join("browser-adapter")
            .join("src")
            .join("service.ts"),
        repo_root,
    ));

    candidates
}

fn infer_asset_root(entry: &Path) -> Option<PathBuf> {
    entry.ancestors().find_map(|ancestor| {
        if ancestor.join("selector-packs").exists() || ancestor.join("packages").exists() {
            Some(ancestor.to_path_buf())
        } else {
            None
        }
    })
}

fn build_adapter_runtime(entry: PathBuf, asset_root: PathBuf) -> Result<AdapterRuntime> {
    let launcher = if is_js_entry(&entry) {
        AdapterLauncher::Node
    } else {
        AdapterLauncher::Tsx(resolve_tsx_runner(&entry).with_context(|| {
            format!(
                "TypeScript browser adapter entry {} requires a local tsx runner",
                entry.display()
            )
        })?)
    };

    Ok(AdapterRuntime {
        entry,
        asset_root,
        launcher,
    })
}

fn resolve_tsx_runner(entry: &Path) -> Option<PathBuf> {
    entry.ancestors().find_map(|ancestor| {
        let candidate = ancestor
            .join("node_modules")
            .join(".bin")
            .join(if cfg!(windows) { "tsx.cmd" } else { "tsx" });
        candidate.exists().then_some(candidate)
    })
}

fn has_playwright_core(entry: &Path) -> bool {
    entry.ancestors().any(|ancestor| {
        ancestor
            .join("node_modules")
            .join("playwright-core")
            .join("package.json")
            .exists()
    })
}

fn command_available(command: &str, version_arg: &str) -> bool {
    Command::new(command)
        .arg(version_arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn node_command() -> &'static str {
    if cfg!(windows) { "node.exe" } else { "node" }
}

fn is_js_entry(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("js"))
        .unwrap_or(false)
}

fn spawn_adapter_reader(
    stdout: ChildStdout,
    tx: mpsc::Sender<Result<AdapterFrame>>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match stdout.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let line = line.trim_end();
                    if line.is_empty() {
                        continue;
                    }
                    let frame = serde_json::from_str(line).context("decode adapter frame");
                    if tx.send(frame).is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx.send(Err(err).context("read adapter frame"));
                    break;
                }
            }
        }
    })
}
