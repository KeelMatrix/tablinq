use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use shared_protocol::{
    ArtifactSummary, ConversationSummary, JobDetail, JobKind, JobStatus, RequestExecutionKind,
    RequestSummary, RequestUsage,
};

pub(crate) fn parse_timestamp(value: String) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
        })
}

pub(crate) fn map_conversation_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ConversationSummary> {
    Ok(ConversationSummary {
        conversation_id: row.get(0)?,
        session_id: row.get(1)?,
        label: row.get(2)?,
        tab_url: row.get(3)?,
        last_prompt: row.get(4)?,
        last_response: row.get(5)?,
        last_seen_at: parse_timestamp(row.get::<_, String>(6)?)?,
    })
}

pub(crate) fn map_artifact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactSummary> {
    Ok(ArtifactSummary {
        artifact_id: row.get(0)?,
        conversation_id: row.get(1)?,
        message_id: row.get(2)?,
        file_name: row.get(3)?,
        content_type: row.get(4)?,
        size_bytes: row.get(5)?,
        sha256: row.get(6)?,
        storage_path: row.get(7)?,
        created_at: parse_timestamp(row.get::<_, String>(8)?)?,
    })
}

pub(crate) fn map_request_summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RequestSummary> {
    map_request_summary_row_from_offset(row, 0)
}

pub(crate) fn map_optional_request_summary_row(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<Option<RequestSummary>> {
    if row.get::<_, Option<String>>(offset)?.is_none() {
        Ok(None)
    } else {
        map_request_summary_row_from_offset(row, offset).map(Some)
    }
}

fn map_request_summary_row_from_offset(
    row: &rusqlite::Row<'_>,
    offset: usize,
) -> rusqlite::Result<RequestSummary> {
    let warnings_index = offset + 24;
    let warnings_json = row.get::<_, String>(warnings_index)?;
    let warnings = serde_json::from_str(&warnings_json).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(
            warnings_index,
            rusqlite::types::Type::Text,
            Box::new(err),
        )
    })?;
    Ok(RequestSummary {
        request_id: row.get(offset)?,
        job_id: row.get(offset + 1)?,
        model_id: row.get(offset + 2)?,
        request_role: row.get(offset + 3)?,
        parent_request_id: row.get(offset + 4)?,
        execution_kind: parse_request_execution_kind(row.get::<_, String>(offset + 5)?.as_str()),
        status: parse_job_status(row.get::<_, String>(offset + 6)?.as_str()),
        cancelled: row.get::<_, i64>(offset + 7)? != 0,
        start_time: parse_timestamp(row.get::<_, String>(offset + 8)?)?,
        end_time: row
            .get::<_, Option<String>>(offset + 9)?
            .map(parse_timestamp)
            .transpose()?,
        duration_ms: row.get(offset + 10)?,
        queue_wait_ms: row.get::<_, i64>(offset + 11)?.max(0) as u64,
        interrupted_at: row
            .get::<_, Option<String>>(offset + 12)?
            .map(parse_timestamp)
            .transpose()?,
        interruption_reason: row.get(offset + 13)?,
        recovered_at: row
            .get::<_, Option<String>>(offset + 14)?
            .map(parse_timestamp)
            .transpose()?,
        recovery_reason: row.get(offset + 15)?,
        failed_at: row
            .get::<_, Option<String>>(offset + 16)?
            .map(parse_timestamp)
            .transpose()?,
        failure_reason: row.get(offset + 17)?,
        recovery_trace: decode_optional_json(
            row.get::<_, Option<String>>(offset + 18)?,
            offset + 18,
        )?,
        recovery_packet: decode_optional_json(
            row.get::<_, Option<String>>(offset + 19)?,
            offset + 19,
        )?,
        usage: RequestUsage {
            status: row.get(offset + 20)?,
            prompt_tokens: row.get(offset + 21)?,
            completion_tokens: row.get(offset + 22)?,
            total_tokens: row.get(offset + 23)?,
        },
        warnings,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_job_detail(
    job_id: String,
    status: JobStatus,
    message: Option<String>,
    conversation_id: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    kind: Option<String>,
    input_text: Option<String>,
    output_text: Option<String>,
    current_url: Option<String>,
    output_path: Option<String>,
    cancel_requested: bool,
    interrupted_at: Option<String>,
    interruption_reason: Option<String>,
    recovered_at: Option<String>,
    recovery_reason: Option<String>,
    failed_at: Option<String>,
    failure_reason: Option<String>,
    recovery_trace_json: Option<String>,
    recovery_packet_json: Option<String>,
    artifacts: Vec<ArtifactSummary>,
) -> Result<JobDetail> {
    Ok(JobDetail {
        job_id,
        kind: kind.as_deref().map(parse_job_kind).unwrap_or(JobKind::Ask),
        status,
        message,
        conversation_id,
        created_at,
        updated_at,
        input_text,
        output_text,
        current_url,
        output_path,
        artifacts,
        cancel_requested,
        interrupted_at: interrupted_at.map(parse_timestamp).transpose()?,
        interruption_reason,
        recovered_at: recovered_at.map(parse_timestamp).transpose()?,
        recovery_reason,
        failed_at: failed_at.map(parse_timestamp).transpose()?,
        failure_reason,
        recovery_trace: decode_optional_json(recovery_trace_json, 18)?,
        recovery_packet: decode_optional_json(recovery_packet_json, 19)?,
    })
}

fn decode_optional_json<T>(value: Option<String>, column: usize) -> rusqlite::Result<Option<T>>
where
    T: DeserializeOwned,
{
    value
        .map(|json| {
            serde_json::from_str(&json).map_err(|err| {
                rusqlite::Error::FromSqlConversionFailure(
                    column,
                    rusqlite::types::Type::Text,
                    Box::new(err),
                )
            })
        })
        .transpose()
}

pub(crate) fn parse_job_status(value: &str) -> JobStatus {
    value.parse().unwrap_or(JobStatus::Failed)
}

fn parse_job_kind(value: &str) -> JobKind {
    match value {
        "research" => JobKind::Research,
        _ => JobKind::Ask,
    }
}

fn parse_request_execution_kind(value: &str) -> RequestExecutionKind {
    match value {
        "research" => RequestExecutionKind::Research,
        _ => RequestExecutionKind::Ask,
    }
}

pub(crate) fn job_status_as_str(value: JobStatus) -> &'static str {
    match value {
        JobStatus::Queued => "queued",
        JobStatus::Starting => "starting",
        JobStatus::Running => "running",
        JobStatus::Interrupted => "interrupted",
        JobStatus::Recovered => "recovered",
        JobStatus::WaitingForSources => "waiting_for_sources",
        JobStatus::Completed => "completed",
        JobStatus::Cancelled => "cancelled",
        JobStatus::Failed => "failed",
        JobStatus::ArtifactReady => "artifact_ready",
    }
}

pub(crate) fn job_kind_as_str(value: JobKind) -> &'static str {
    match value {
        JobKind::Ask => "ask",
        JobKind::Research => "research",
    }
}

pub(crate) fn request_execution_kind_as_str(value: RequestExecutionKind) -> &'static str {
    match value {
        RequestExecutionKind::Ask => "ask",
        RequestExecutionKind::Research => "research",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_status_maps_to_failed() {
        assert_eq!(parse_job_status("unexpected"), JobStatus::Failed);
    }

    #[test]
    fn request_execution_kind_roundtrips_database_values() {
        assert_eq!(
            request_execution_kind_as_str(RequestExecutionKind::Ask),
            "ask"
        );
        assert_eq!(
            request_execution_kind_as_str(RequestExecutionKind::Research),
            "research"
        );
    }
}
