mod artifact_operations;
mod ask;
mod chatgpt_message_limit;
mod conversation_inspection;
mod job_operations;
mod lifecycle;
mod native_upload_limit;
mod operations;
mod provider_failure_recovery;
mod recovery;
mod recovery_manifest;
mod research;
mod support;

use crate::browser::BrowserManager;
use crate::config::AppConfig;
use crate::db::Database;
use anyhow::Result;
pub use ask::PreparedAskJob;
use chrono::Utc;
pub use research::PreparedResearchJob;
use shared_protocol::{
    AskCommand, AskResult, BrowserOpenRequest, EventPayload, ResearchCommand, ResearchResult,
};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
pub use support::error_code_for_message;
use support::lock_browser;

const ARTIFACT_RETENTION_LIMIT: usize = 100;

#[derive(Debug, Default)]
struct ExecutionRecoveryState {
    interrupted: bool,
    recovered: bool,
}

#[derive(Clone)]
pub struct CoreService {
    config: AppConfig,
    started_at: chrono::DateTime<Utc>,
    browser: Arc<Mutex<BrowserManager>>,
    cancel_registry: Arc<Mutex<HashSet<String>>>,
}

impl CoreService {
    pub fn new(config: AppConfig) -> Result<Self> {
        let db = Database::open(&config.paths.db_path)?;
        db.fail_incomplete_jobs("daemon restarted before the job finished", Utc::now())?;

        Ok(Self {
            config: config.clone(),
            started_at: Utc::now(),
            browser: Arc::new(Mutex::new(BrowserManager::new(&config))),
            cancel_registry: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn warm_browser_if_enabled(&self) -> Result<()> {
        if self.config.warm_mode() {
            lock_browser(&self.browser)?.open(BrowserOpenRequest {
                target_url: None,
                browser_path: None,
                headless: None,
            })?;
        }
        Ok(())
    }

    pub fn ask<F>(&self, command: AskCommand, mut on_event: F) -> Result<AskResult>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let prepared = self.prepare_ask(command)?;
        on_event(prepared.queued_event())?;
        self.execute_prepared_ask(&prepared, on_event)
    }

    pub fn research<F>(&self, command: ResearchCommand, mut on_event: F) -> Result<ResearchResult>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let prepared = self.prepare_research(command)?;
        on_event(prepared.queued_event())?;
        self.execute_prepared_research(&prepared, on_event)
    }
}
