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

pub(in crate::core) fn apply_research_update<F>(
    db: &Database,
    job_id: &str,
    created_at: chrono::DateTime<Utc>,
    conversation_id: &mut Option<String>,
    current_url: &mut Option<String>,
    report_text: &mut String,
    next_seq: &mut u64,
    update: ResearchUpdate,
    on_event: &mut F,
) -> Result<()>
where
    F: FnMut(EventPayload) -> Result<()>,
{
    let updated_at = Utc::now();
    if let Some(value) = update.conversation_id.clone() {
        *conversation_id = Some(value);
    }
    if let Some(value) = update.current_url.clone() {
        *current_url = Some(value);
    }

    db.insert_job(
        job_id,
        update.status,
        update.message.as_deref(),
        update
            .conversation_id
            .as_deref()
            .or(conversation_id.as_deref()),
        created_at,
        updated_at,
    )?;

    if let Some(chunk_text) = update.chunk_text.clone() {
        if matches!(
            update.chunk_kind.unwrap_or(StreamChunkKind::Delta),
            StreamChunkKind::Delta
        ) {
            report_text.push_str(&chunk_text);
        }
        on_event(EventPayload::StreamChunk(StreamChunk {
            job_id: job_id.to_owned(),
            conversation_id: update.conversation_id.clone().or(conversation_id.clone()),
            message_id: update.message_id.clone(),
            seq: *next_seq,
            kind: update.chunk_kind.unwrap_or(StreamChunkKind::Delta),
            text: chunk_text,
        }))?;
        *next_seq += 1;
    }

    db.update_job_output(
        job_id,
        if report_text.is_empty() {
            None
        } else {
            Some(report_text.as_str())
        },
        current_url.as_deref(),
        None,
    )?;
    on_event(EventPayload::JobState(JobState {
        job_id: job_id.to_owned(),
        status: update.status,
        message: update.message,
        conversation_id: update.conversation_id.or(conversation_id.clone()),
        updated_at,
        artifacts: vec![],
    }))
}

pub(in crate::core) fn maybe_write_research_output(
    command: &ResearchCommand,
    job_id: &str,
    report_text: &str,
) -> Result<Option<String>> {
    let Some(output_dir) = command.output_dir.as_deref() else {
        return Ok(None);
    };

    let output_dir = PathBuf::from(output_dir);
    fs::create_dir_all(&output_dir).with_context(|| format!("create {}", output_dir.display()))?;
    let destination = unique_destination_path(
        &output_dir,
        Path::new(&format!("{}-report.md", sanitize_file_stem(job_id))),
    );
    fs::write(&destination, report_text)
        .with_context(|| format!("write {}", destination.display()))?;
    Ok(Some(destination.display().to_string()))
}

fn sanitize_file_stem(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

pub(in crate::core) fn unique_destination_path(
    destination_dir: &Path,
    file_name: &Path,
) -> PathBuf {
    let candidate = destination_dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let stem = file_name
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("artifact");
    let extension = file_name.extension().and_then(|value| value.to_str());

    for index in 2.. {
        let name = match extension {
            Some(extension) => format!("{stem}-{index}.{extension}"),
            None => format!("{stem}-{index}"),
        };
        let candidate = destination_dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("destination path generation should always return");
}
