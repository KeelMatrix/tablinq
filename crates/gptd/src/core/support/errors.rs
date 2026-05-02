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

pub fn error_code_for_message(message: &str) -> ErrorCode {
    if is_provider_generation_failure_message(message) {
        ErrorCode::ProviderGenerationFailedNonretryable
    } else if message.contains("provider_conversation_load_failed")
        || message.contains("did not load conversation")
    {
        ErrorCode::ProviderConversationLoadFailed
    } else if message.contains("attach debug endpoint is not reachable")
        || message.contains("attach_endpoint_unavailable")
    {
        ErrorCode::AttachEndpointUnavailable
    } else if message.contains("profile locked") || message.contains("SingletonLock") {
        ErrorCode::ProfileLocked
    } else if message.contains("auth required") {
        ErrorCode::AuthRequired
    } else if message.contains("browser launch failed")
        || message.contains("spawn browser adapter")
        || message.contains("supported browser executable")
    {
        ErrorCode::BrowserLaunch
    } else if message.contains("capability missing") {
        ErrorCode::CapabilityMissing
    } else if message.contains("research job failed")
        || message.contains("research failed")
        || message.contains("research cancelled")
    {
        ErrorCode::ResearchJobFailed
    } else if message.contains("upload failed") {
        ErrorCode::UploadFailed
    } else if message.contains("download failed") {
        ErrorCode::DownloadFailed
    } else if message.contains("ui drift") {
        ErrorCode::UiDrift
    } else if message.contains("timed out") {
        ErrorCode::ActionTimeout
    } else {
        ErrorCode::Transport
    }
}

pub(in crate::core) fn is_cancelled_message(message: &str) -> bool {
    message.contains("cancelled")
}
