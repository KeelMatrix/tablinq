use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use shared_protocol::{
    JobStatus, RequestHistory, RequestListEntry, RequestListPage, RequestSummary,
};

use super::{
    REQUEST_HISTORY_LIMIT,
    row_mapping::{job_status_as_str, map_request_summary_row, request_execution_kind_as_str},
};

pub(crate) fn upsert_summary(conn: &Connection, summary: &RequestSummary) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO request_history (
            request_id, job_id, model_id, request_role, parent_request_id, execution_kind,
            status, cancelled, start_time, end_time, duration_ms, queue_wait_ms,
            interrupted_at, interruption_reason, recovered_at, recovery_reason, failed_at,
            failure_reason, recovery_trace_json, recovery_packet_json, usage_status,
            prompt_tokens, completion_tokens, total_tokens, warnings_json
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
            ?19, ?20, ?21, ?22, ?23, ?24, ?25
        )
        ON CONFLICT(request_id) DO UPDATE SET
            job_id = excluded.job_id,
            model_id = excluded.model_id,
            request_role = excluded.request_role,
            parent_request_id = excluded.parent_request_id,
            execution_kind = excluded.execution_kind,
            status = excluded.status,
            cancelled = excluded.cancelled,
            start_time = excluded.start_time,
            end_time = excluded.end_time,
            duration_ms = excluded.duration_ms,
            queue_wait_ms = excluded.queue_wait_ms,
            interrupted_at = excluded.interrupted_at,
            interruption_reason = excluded.interruption_reason,
            recovered_at = excluded.recovered_at,
            recovery_reason = excluded.recovery_reason,
            failed_at = excluded.failed_at,
            failure_reason = excluded.failure_reason,
            recovery_trace_json = excluded.recovery_trace_json,
            recovery_packet_json = excluded.recovery_packet_json,
            usage_status = excluded.usage_status,
            prompt_tokens = excluded.prompt_tokens,
            completion_tokens = excluded.completion_tokens,
            total_tokens = excluded.total_tokens,
            warnings_json = excluded.warnings_json
        "#,
        params![
            summary.request_id,
            summary.job_id,
            summary.model_id,
            summary.request_role,
            summary.parent_request_id,
            request_execution_kind_as_str(summary.execution_kind),
            job_status_as_str(summary.status),
            if summary.cancelled { 1 } else { 0 },
            summary.start_time.to_rfc3339(),
            summary.end_time.map(|value| value.to_rfc3339()),
            summary.duration_ms,
            summary.queue_wait_ms,
            summary.interrupted_at.map(|value| value.to_rfc3339()),
            summary.interruption_reason,
            summary.recovered_at.map(|value| value.to_rfc3339()),
            summary.recovery_reason,
            summary.failed_at.map(|value| value.to_rfc3339()),
            summary.failure_reason,
            summary
                .recovery_trace
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
            summary
                .recovery_packet
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
            summary.usage.status,
            summary.usage.prompt_tokens,
            summary.usage.completion_tokens,
            summary.usage.total_tokens,
            serde_json::to_string(&summary.warnings)?,
        ],
    )?;
    prune_history(conn, REQUEST_HISTORY_LIMIT)?;
    Ok(())
}

pub(crate) fn find_summary(conn: &Connection, request_id: &str) -> Result<Option<RequestSummary>> {
    conn.query_row(
        r#"
        SELECT request_id,
               job_id,
               model_id,
               request_role,
               parent_request_id,
               execution_kind,
               status,
               cancelled,
               start_time,
               end_time,
               duration_ms,
               queue_wait_ms,
               interrupted_at,
               interruption_reason,
               recovered_at,
               recovery_reason,
               failed_at,
               failure_reason,
               recovery_trace_json,
               recovery_packet_json,
               usage_status,
               prompt_tokens,
               completion_tokens,
               total_tokens,
               warnings_json
        FROM request_history
        WHERE request_id = ?1
        "#,
        [request_id],
        map_request_summary_row,
    )
    .optional()
    .map_err(Into::into)
}

pub(crate) fn list_summaries(conn: &Connection, limit: usize) -> Result<RequestHistory> {
    let mut statement = conn.prepare(
        r#"
        SELECT request_id,
               job_id,
               model_id,
               request_role,
               parent_request_id,
               execution_kind,
               status,
               cancelled,
               start_time,
               end_time,
               duration_ms,
               queue_wait_ms,
               interrupted_at,
               interruption_reason,
               recovered_at,
               recovery_reason,
               failed_at,
               failure_reason,
               recovery_trace_json,
               recovery_packet_json,
               usage_status,
               prompt_tokens,
               completion_tokens,
               total_tokens,
               warnings_json
        FROM request_history
        ORDER BY start_time DESC
        LIMIT ?1
        "#,
    )?;
    let rows = statement.query_map([limit as i64], map_request_summary_row)?;
    Ok(RequestHistory {
        requests: rows.collect::<std::result::Result<Vec<_>, _>>()?,
    })
}

pub(crate) fn list_page(
    conn: &Connection,
    status: Option<JobStatus>,
    limit: usize,
    offset: usize,
) -> Result<RequestListPage> {
    let mut statement = conn.prepare(
        r#"
        SELECT request_id,
               job_id,
               model_id,
               request_role,
               parent_request_id,
               execution_kind,
               status,
               cancelled,
               start_time,
               end_time,
               duration_ms,
               queue_wait_ms,
               interrupted_at,
               interruption_reason,
               recovered_at,
               recovery_reason,
               failed_at,
               failure_reason,
               recovery_trace_json,
               recovery_packet_json,
               usage_status,
               prompt_tokens,
               completion_tokens,
               total_tokens,
               warnings_json
        FROM request_history
        WHERE (?1 IS NULL OR status = ?1)
        ORDER BY start_time DESC
        LIMIT ?2 OFFSET ?3
        "#,
    )?;
    let query_limit = limit.saturating_add(1) as i64;
    let status_value = status.as_ref().map(JobStatus::as_str);
    let rows = statement.query_map(
        params![status_value, query_limit, offset as i64],
        map_request_summary_row,
    )?;
    let mut requests = rows
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .map(RequestListEntry::from)
        .collect::<Vec<_>>();
    let next_offset = if requests.len() > limit {
        requests.truncate(limit);
        Some(offset + limit)
    } else {
        None
    };
    Ok(RequestListPage {
        requests,
        limit,
        offset,
        next_offset,
        status,
    })
}

fn prune_history(conn: &Connection, limit: usize) -> Result<()> {
    conn.execute(
        r#"
        DELETE FROM request_history
        WHERE request_id NOT IN (
            SELECT request_id
            FROM request_history
            ORDER BY start_time DESC
            LIMIT ?1
        )
        "#,
        [limit as i64],
    )?;
    Ok(())
}
