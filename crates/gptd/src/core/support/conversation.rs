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

pub(in crate::core) fn resolve_conversation_target_url(base_url: &str, conversation_id: &str) -> String {
    let normalized_conversation_id = conversation_id.trim();
    if normalized_conversation_id.is_empty() {
        return base_url.to_owned();
    }

    let Ok(mut url) = Url::parse(base_url) else {
        return base_url.to_owned();
    };
    let mut segments = url
        .path_segments()
        .map(|parts| parts.map(str::to_owned).collect::<Vec<_>>())
        .unwrap_or_default();
    if segments.len() >= 2
        && segments[segments.len() - 2].eq_ignore_ascii_case("c")
        && !segments[segments.len() - 1].is_empty()
    {
        segments.truncate(segments.len() - 2);
    }

    let Ok(mut path_segments) = url.path_segments_mut() else {
        return base_url.to_owned();
    };
    path_segments.clear();
    for segment in &segments {
        path_segments.push(segment);
    }
    path_segments.push("c");
    path_segments.push(normalized_conversation_id);
    drop(path_segments);

    url.to_string()
}

pub(in crate::core) fn conversation_id_from_url(raw_url: &str) -> Option<String> {
    let marker = "/c/";
    let start = raw_url.find(marker)? + marker.len();
    let rest = &raw_url[start..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let conversation_id = rest[..end].trim();
    if conversation_id.is_empty() {
        None
    } else {
        Some(conversation_id.to_owned())
    }
}

pub(in crate::core) fn browser_session_identity(snapshot: &BrowserStatusSnapshot) -> (String, String) {
    if snapshot.cdp_attached
        && let Some(tab_id) = snapshot
            .target_tab_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
    {
        let short_id = &tab_id[..tab_id.len().min(8)];
        return (
            format!("browser-tab-{tab_id}"),
            format!("Browser tab {short_id}"),
        );
    }

    ("browser-primary".to_owned(), "Browser session".to_owned())
}
