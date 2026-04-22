mod adapter_process;
mod error;
mod protocol;
mod recovery;
mod runtime;

use crate::config::AppConfig;
use crate::logging::JsonLogger;
use adapter_process::AdapterProcess;
use anyhow::{Context, Result};
pub use error::{
    ExecutionErrorMetadata, adapter_error_code, execution_error, execution_error_metadata,
};
use protocol::AdapterRequest;
use recovery::{
    derive_recovery_conversation_id, idle_snapshot, is_recoverable_adapter_error,
    normalize_snapshot, resolve_ask_timeout_ms, should_reset_after_failure,
};
use serde::Deserialize;
use serde_json::json;
use shared_protocol::{
    AskCommand, AskResult, BrowserOpenRequest, BrowserStatusSnapshot, CapabilityReport,
    EventPayload, JobStatus, ResearchCommand, ResearchResult, SelectorPackSummary, StrategyAttempt,
    StrategyTelemetryEntry, StreamChunkKind,
};
use std::cell::{Cell, RefCell};
use std::fs;
use std::path::Path;

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

pub fn browser_integration_prereqs_available() -> bool {
    runtime::browser_integration_prereqs_available()
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
                force_reload: false,
            }) {
            Ok(snapshot) => {
                let snapshot = normalize_snapshot(&self.config, snapshot);
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

    pub fn reload_target_page(&mut self) -> Result<BrowserStatusSnapshot> {
        self.ensure_process()?;

        let target_url = self.current_target();
        let browser_path = self.last_snapshot.browser_path.clone();
        let request_browser_path = browser_path.clone();
        match self
            .process
            .as_mut()
            .expect("browser adapter process missing")
            .request(AdapterRequest::Open {
                target_url,
                browser_path,
                headless: false,
                force_reload: true,
            }) {
            Ok(snapshot) => {
                let snapshot = normalize_snapshot(&self.config, snapshot);
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
        let latest_state = RefCell::new(None::<RequestLifecycleUpdate>);
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
        let mut state_forwarding = |update: RequestLifecycleUpdate| {
            latest_state.replace(Some(update.clone()));
            on_state(update)
        };

        match self
            .process
            .as_mut()
            .expect("browser adapter process missing")
            .ask(
                job_id,
                command,
                resolve_ask_timeout_ms(&self.config, command),
                &mut forwarding,
                &mut state_forwarding,
                &mut should_cancel,
            ) {
            Ok((snapshot, result)) => {
                self.last_snapshot = snapshot;
                Ok(result)
            }
            Err(err) => {
                if is_recoverable_adapter_error(&err.to_string()) {
                    let recovery_conversation_id = derive_recovery_conversation_id(
                        command.conversation_id.as_deref(),
                        latest_state.borrow().as_ref(),
                        self.last_snapshot.current_url.as_deref(),
                    );
                    let recovery_current_url = latest_state
                        .borrow()
                        .as_ref()
                        .and_then(|update| update.current_url.clone())
                        .or_else(|| self.last_snapshot.current_url.clone());
                    let mut recovery_command = command.clone();
                    if recovery_command.conversation_id.is_none() {
                        recovery_command.conversation_id = recovery_conversation_id.clone();
                    }
                    if recovery_command.conversation_id.is_some() {
                        recovery_command.new_conversation = false;
                    }
                    state_forwarding(RequestLifecycleUpdate {
                        status: JobStatus::Interrupted,
                        message: format!("ask interrupted: {err:#}"),
                        conversation_id: recovery_conversation_id,
                        message_id: None,
                        current_url: recovery_current_url,
                        streaming_safe: false,
                    })?;
                    self.relaunch_for_recovery()?;
                    let (snapshot, result) = self
                        .process
                        .as_mut()
                        .expect("browser adapter process missing")
                        .recover_ask(
                            job_id,
                            &recovery_command,
                            resolve_ask_timeout_ms(&self.config, &recovery_command),
                            emitted_text.borrow().as_str(),
                            next_seq.get(),
                            &mut forwarding,
                            &mut state_forwarding,
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
        let latest_state = RefCell::new(None::<RequestLifecycleUpdate>);
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
        let mut state_forwarding = |update: RequestLifecycleUpdate| {
            latest_state.replace(Some(update.clone()));
            on_state(update)
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
                &mut state_forwarding,
                &mut should_cancel,
            ) {
            Ok((snapshot, result)) => {
                self.last_snapshot = snapshot;
                Ok(result)
            }
            Err(err) => {
                if is_recoverable_adapter_error(&err.to_string()) {
                    let recovery_conversation_id = derive_recovery_conversation_id(
                        command.conversation_id.as_deref(),
                        latest_state.borrow().as_ref(),
                        self.last_snapshot.current_url.as_deref(),
                    );
                    let recovery_current_url = latest_state
                        .borrow()
                        .as_ref()
                        .and_then(|update| update.current_url.clone())
                        .or_else(|| self.last_snapshot.current_url.clone());
                    let mut recovery_command = command.clone();
                    if recovery_command.conversation_id.is_none() {
                        recovery_command.conversation_id = recovery_conversation_id.clone();
                    }
                    if recovery_command.conversation_id.is_some() {
                        recovery_command.new_conversation = false;
                    }
                    state_forwarding(RequestLifecycleUpdate {
                        status: JobStatus::Interrupted,
                        message: format!("research interrupted: {err:#}"),
                        conversation_id: recovery_conversation_id,
                        message_id: None,
                        current_url: recovery_current_url,
                        streaming_safe: false,
                    })?;
                    self.relaunch_for_recovery()?;
                    let (snapshot, result) = self
                        .process
                        .as_mut()
                        .expect("browser adapter process missing")
                        .recover_research(
                            job_id,
                            &recovery_command,
                            self.config.research_timeout_ms(),
                            emitted_text.borrow().as_str(),
                            next_seq.get(),
                            &mut forwarding,
                            &mut state_forwarding,
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
                let snapshot = normalize_snapshot(&self.config, snapshot);
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
            self.log_process_event(
                "browser adapter teardown starting",
                json!({
                    "teardown_in_progress": true,
                    "target_url": self.last_snapshot.target_url,
                }),
            );
            process.terminate();
            self.log_process_event(
                "browser adapter teardown finished",
                json!({
                    "teardown_in_progress": false,
                    "target_url": self.last_snapshot.target_url,
                }),
            );
        }
    }

    fn relaunch_for_recovery(&mut self) -> Result<()> {
        self.log_process_event(
            "browser adapter recovery relaunch starting",
            json!({
                "teardown_in_progress": self.process.is_some(),
                "target_url": self.last_snapshot.target_url,
            }),
        );
        self.teardown_process();
        self.ensure_process()?;
        self.log_process_event(
            "browser adapter recovery relaunch finished",
            json!({
                "teardown_in_progress": false,
                "adapter_running": self.process.is_some(),
                "target_url": self.last_snapshot.target_url,
            }),
        );
        Ok(())
    }

    fn current_target(&self) -> String {
        self.last_snapshot.target_url.clone()
    }

    fn log_process_event(&self, message: &str, fields: serde_json::Value) {
        if let Ok(logger) = JsonLogger::new(&self.config.paths.log_path) {
            let _ = logger.info(message, fields);
        }
    }
}

impl Drop for BrowserManager {
    fn drop(&mut self) {
        self.teardown_process();
    }
}
