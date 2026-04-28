#![allow(unused_imports)]
use super::super::{
    CoreService, ExecutionRecoveryState, recovery::is_provider_generation_failure_message,
};
use crate::browser::{
    BrowserDiagnosticsSnapshot, BrowserManager, RequestLifecycleUpdate, ResearchUpdate,
};
use crate::config::AppConfig;
use crate::db::Database;
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use shared_protocol::{
    BrowserConnectionMode, BrowserStatusSnapshot, ErrorCode, EventPayload, HealthCheck,
    HealthStatus, JobState, JobStatus, ResearchCommand, SelectorPackSummary, StreamChunk,
    StreamChunkKind,
};
use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use url::Url;

pub(in crate::core) fn lock_browser<'a>(
    browser: &'a Arc<Mutex<BrowserManager>>,
) -> Result<MutexGuard<'a, BrowserManager>> {
    browser
        .lock()
        .map_err(|_| anyhow!("browser manager lock poisoned"))
}

pub(in crate::core) fn try_lock_browser<'a>(
    browser: &'a Arc<Mutex<BrowserManager>>,
) -> Result<Option<MutexGuard<'a, BrowserManager>>> {
    match browser.try_lock() {
        Ok(guard) => Ok(Some(guard)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Poisoned(_)) => Err(anyhow!("browser manager lock poisoned")),
    }
}

pub(in crate::core) fn lock_cancel_registry<'a>(
    registry: &'a Arc<Mutex<HashSet<String>>>,
) -> Result<MutexGuard<'a, HashSet<String>>> {
    registry
        .lock()
        .map_err(|_| anyhow!("cancel registry lock poisoned"))
}

pub(in crate::core) fn browser_busy_message() -> &'static str {
    "browser busy: another ask or research request is already using the browser session"
}
