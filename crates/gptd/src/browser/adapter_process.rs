use super::protocol::{
    AdapterFrame, AdapterRequest, AdapterResearchUpdate, AdapterStreamChunk,
    map_request_state_update,
};
use super::runtime::{
    AdapterLauncher, browser_connection_mode_name, node_command, resolve_adapter_runtime,
};
use super::{
    BrowserDiagnosticsSnapshot, ExecutionErrorMetadata, RequestLifecycleUpdate, ResearchUpdate,
    adapter_error_code, execution_error,
};
use crate::config::AppConfig;
use anyhow::{Context, Result, anyhow, bail};
use shared_protocol::{
    AskCommand, AskResult, BrowserStatusSnapshot, EventPayload, ResearchCommand, ResearchResult,
    StreamChunk, StreamChunkKind,
};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ADAPTER_CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
const ASK_RESULT_GRACE_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) struct AdapterProcess {
    child: Child,
    stdin: ChildStdin,
    frames: Receiver<Result<AdapterFrame>>,
    reader_thread: Option<JoinHandle<()>>,
    submit_jitter_min_ms: u64,
    submit_jitter_max_ms: u64,
}

impl AdapterProcess {
    pub(super) fn spawn(config: &AppConfig) -> Result<Self> {
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
            .arg("--connection-mode")
            .arg(browser_connection_mode_name(
                config.browser_connection_mode(),
            ))
            .arg("--debug-endpoint")
            .arg(config.browser_debug_endpoint())
            .arg("--extension-required")
            .arg(if config.browser_extension_required() {
                "true"
            } else {
                "false"
            })
            .arg("--auto-open-target-tab")
            .arg(if config.browser_auto_open_target_tab() {
                "true"
            } else {
                "false"
            })
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
            submit_jitter_min_ms: config.browser_submit_jitter_min_ms(),
            submit_jitter_max_ms: config.browser_submit_jitter_max_ms(),
        })
    }

    pub(super) fn request(&mut self, request: AdapterRequest) -> Result<BrowserStatusSnapshot> {
        self.write_request(&request, "encode adapter request", "write adapter request")?;

        let response = self.next_frame_blocking("response")?;
        match response {
            AdapterFrame::Response {
                ok,
                state,
                error,
                error_code,
            } => {
                if ok {
                    Ok(state)
                } else {
                    Err(adapter_frame_error(
                        error_code,
                        error,
                        state.current_url,
                        "browser adapter returned an unknown error",
                    ))
                }
            }
            other => bail!("unexpected adapter frame for request: {other:?}"),
        }
    }

    pub(super) fn diagnostics(
        &mut self,
        bundle_dir: Option<&Path>,
    ) -> Result<BrowserDiagnosticsSnapshot> {
        let request = AdapterRequest::Diagnostics {
            bundle_dir: bundle_dir.map(|path| path.display().to_string()),
        };
        self.write_request(
            &request,
            "encode adapter diagnostics request",
            "write adapter diagnostics request",
        )?;

        let response = self.next_frame_blocking("diagnostics response")?;
        match response {
            AdapterFrame::Diagnostics {
                ok,
                state: _state,
                diagnostics,
                error,
            } => {
                if ok {
                    (*diagnostics).context("adapter diagnostics finished without a result")
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

    pub(super) fn ask<F>(
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
                new_conversation: command.new_conversation,
                timeout_ms: Some(timeout_ms),
                submit_jitter_ms: Some(self.next_submit_jitter_ms()),
                mode: Some(command.mode),
                model_class: command.model_class,
                effort: command.effort,
                files: command.files.clone(),
                emitted_text: None,
                next_seq: None,
            },
            "ask",
            timeout_ms,
            on_event,
            on_state,
            should_cancel,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn recover_ask<F>(
        &mut self,
        job_id: &str,
        command: &AskCommand,
        timeout_ms: u64,
        emitted_text: &str,
        next_seq: u64,
        known_conversation_id: Option<&str>,
        known_message_id: Option<&str>,
        latest_update: Option<&RequestLifecycleUpdate>,
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
                new_conversation: command.new_conversation,
                timeout_ms: Some(timeout_ms),
                submit_jitter_ms: Some(self.next_submit_jitter_ms()),
                mode: Some(command.mode),
                model_class: command.model_class,
                effort: command.effort,
                files: command.files.clone(),
                emitted_text: Some(emitted_text.to_owned()),
                next_seq: Some(next_seq),
                known_conversation_id: known_conversation_id.map(ToOwned::to_owned),
                known_message_id: known_message_id.map(ToOwned::to_owned),
                baseline_conversation_id: latest_update
                    .and_then(|update| update.baseline_conversation_id.clone()),
                baseline_message_id: latest_update
                    .and_then(|update| update.baseline_message_id.clone()),
                baseline_assistant_message_count: latest_update
                    .and_then(|update| update.baseline_assistant_message_count),
                baseline_text: latest_update.and_then(|update| update.baseline_text.clone()),
            },
            "ask recovery",
            timeout_ms,
            on_event,
            on_state,
            should_cancel,
        )
    }

    fn ask_impl<F>(
        &mut self,
        request: AdapterRequest,
        context: &str,
        timeout_ms: u64,
        on_event: &mut F,
        on_state: &mut impl FnMut(RequestLifecycleUpdate) -> Result<()>,
        should_cancel: &mut impl FnMut() -> bool,
    ) -> Result<(BrowserStatusSnapshot, AskResult)>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        self.write_request(
            &request,
            "encode adapter ask request",
            "write adapter ask request",
        )?;
        let deadline =
            Instant::now() + Duration::from_millis(timeout_ms) + ASK_RESULT_GRACE_TIMEOUT;

        loop {
            let frame =
                self.next_frame_interruptible_until(context, should_cancel, deadline, timeout_ms)?;
            match frame {
                AdapterFrame::StreamChunk { chunk } => {
                    on_event(EventPayload::StreamChunk(map_stream_chunk(chunk)))?;
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
                            (*result).context("adapter ask finished without a result")?,
                        ));
                    }
                    return Err(adapter_frame_error(
                        None,
                        error,
                        state.current_url,
                        "browser adapter returned an unknown ask error",
                    ));
                }
                AdapterFrame::Response {
                    ok: false,
                    state,
                    error,
                    error_code,
                } => {
                    return Err(adapter_frame_error(
                        error_code,
                        error,
                        state.current_url,
                        "browser adapter returned an unknown ask error",
                    ));
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

    pub(super) fn research<F, G>(
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
                submit_jitter_ms: Some(self.next_submit_jitter_ms()),
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
    pub(super) fn recover_research<F, G>(
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
                submit_jitter_ms: Some(self.next_submit_jitter_ms()),
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
        self.write_request(
            &request,
            "encode adapter research request",
            "write adapter research request",
        )?;

        loop {
            let frame = self.next_frame_interruptible(context, should_cancel)?;
            match frame {
                AdapterFrame::ResearchUpdate { update } => {
                    on_update(map_research_update(update))?;
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
                            (*result).context("adapter research finished without a result")?,
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

    fn write_request(
        &mut self,
        request: &AdapterRequest,
        encode_context: &str,
        write_context: &str,
    ) -> Result<()> {
        let raw = serde_json::to_string(request).context(encode_context.to_owned())?;
        self.stdin
            .write_all(raw.as_bytes())
            .context(write_context.to_owned())?;
        self.stdin
            .write_all(b"\n")
            .context("write adapter newline")?;
        self.stdin
            .flush()
            .context(format!("flush {write_context}"))?;
        Ok(())
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

    fn next_frame_interruptible_until(
        &mut self,
        context: &str,
        should_cancel: &mut impl FnMut() -> bool,
        deadline: Instant,
        timeout_ms: u64,
    ) -> Result<AdapterFrame> {
        loop {
            if should_cancel() {
                self.terminate();
                bail!("{context} cancelled");
            }
            if Instant::now() >= deadline {
                self.terminate();
                bail!(
                    "browser adapter timed out after {timeout_ms} ms while waiting for {context}"
                );
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

    pub(super) fn terminate(&mut self) {
        terminate_child_process_tree(&mut self.child);
        let _ = self.child.wait();
        if let Some(reader_thread) = self.reader_thread.take() {
            let _ = reader_thread.join();
        }
    }

    fn next_submit_jitter_ms(&self) -> u64 {
        bounded_submit_jitter_ms(self.submit_jitter_min_ms, self.submit_jitter_max_ms)
    }
}

#[cfg(windows)]
fn terminate_child_process_tree(child: &mut Child) {
    let _ = Command::new("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

#[cfg(not(windows))]
fn terminate_child_process_tree(child: &mut Child) {
    let _ = child.kill();
}

fn bounded_submit_jitter_ms(min_ms: u64, max_ms: u64) -> u64 {
    if max_ms <= min_ms {
        return min_ms;
    }

    let span = max_ms - min_ms;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    min_ms + (u64::from(now.subsec_nanos()) % (span + 1))
}

fn adapter_frame_error(
    error_code: Option<String>,
    error: Option<String>,
    current_url: Option<String>,
    fallback_message: &str,
) -> anyhow::Error {
    let message = error.unwrap_or_else(|| fallback_message.to_owned());
    if let Some(code) = error_code.as_deref().and_then(adapter_error_code) {
        return execution_error(ExecutionErrorMetadata::new(code, message, current_url));
    }
    anyhow!(message)
}

fn map_stream_chunk(chunk: AdapterStreamChunk) -> StreamChunk {
    StreamChunk {
        job_id: chunk.job_id,
        conversation_id: chunk.conversation_id,
        message_id: chunk.message_id,
        seq: chunk.seq,
        kind: if chunk.kind == "done" {
            StreamChunkKind::Done
        } else {
            StreamChunkKind::Delta
        },
        text: chunk.text,
    }
}

fn map_research_update(update: AdapterResearchUpdate) -> ResearchUpdate {
    ResearchUpdate {
        status: update.status,
        message: update.message,
        conversation_id: update.conversation_id,
        message_id: update.message_id,
        current_url: update.current_url,
        chunk_kind: update.chunk_kind,
        chunk_text: update.chunk_text,
    }
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
