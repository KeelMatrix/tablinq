use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use shared_protocol::{
    JobDetail, JobKind, JobList, JobStatus, JobSummary, RecoveryPacket, RecoveryTrace,
};

use super::{
    artifacts,
    row_mapping::{
        build_job_detail, job_kind_as_str, job_status_as_str, parse_job_status, parse_timestamp,
    },
};

pub(crate) fn list(conn: &Connection) -> Result<JobList> {
    let mut statement = conn.prepare(
        "SELECT job_id, status, message, conversation_id, created_at, updated_at FROM jobs ORDER BY updated_at DESC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(JobSummary {
            job_id: row.get(0)?,
            status: parse_job_status(row.get::<_, String>(1)?.as_str()),
            message: row.get(2)?,
            conversation_id: row.get(3)?,
            created_at: parse_timestamp(row.get::<_, String>(4)?)?,
            updated_at: parse_timestamp(row.get::<_, String>(5)?)?,
        })
    })?;

    let jobs = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(JobList { jobs })
}

pub(crate) fn insert(
    conn: &Connection,
    job_id: &str,
    status: JobStatus,
    message: Option<&str>,
    conversation_id: Option<&str>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
) -> Result<()> {
    conn.execute(
        r#"
        INSERT OR REPLACE INTO jobs (job_id, status, message, conversation_id, created_at, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        params![
            job_id,
            job_status_as_str(status),
            message,
            conversation_id,
            created_at.to_rfc3339(),
            updated_at.to_rfc3339()
        ],
    )?;
    Ok(())
}

pub(crate) fn upsert_detail(
    conn: &Connection,
    job_id: &str,
    kind: JobKind,
    input_text: Option<&str>,
    output_path: Option<&str>,
) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO job_details (job_id, job_kind, input_text, output_text, current_url, output_path, cancel_requested)
        VALUES (?1, ?2, ?3, NULL, NULL, ?4, 0)
        ON CONFLICT(job_id) DO UPDATE SET
            job_kind = excluded.job_kind,
            input_text = excluded.input_text,
            output_path = COALESCE(excluded.output_path, job_details.output_path)
        "#,
        params![job_id, job_kind_as_str(kind), input_text, output_path],
    )?;
    Ok(())
}

pub(crate) fn update_output(
    conn: &Connection,
    job_id: &str,
    output_text: Option<&str>,
    current_url: Option<&str>,
    output_path: Option<&str>,
) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO job_details (job_id, job_kind, input_text, output_text, current_url, output_path, cancel_requested)
        VALUES (?1, 'ask', NULL, ?2, ?3, ?4, 0)
        ON CONFLICT(job_id) DO UPDATE SET
            output_text = COALESCE(excluded.output_text, job_details.output_text),
            current_url = COALESCE(excluded.current_url, job_details.current_url),
            output_path = COALESCE(excluded.output_path, job_details.output_path)
        "#,
        params![job_id, output_text, current_url, output_path],
    )?;
    Ok(())
}

pub(crate) fn set_cancel_requested(
    conn: &Connection,
    job_id: &str,
    cancel_requested: bool,
) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO job_details (job_id, job_kind, input_text, output_text, current_url, output_path, cancel_requested)
        VALUES (?1, 'ask', NULL, NULL, NULL, NULL, ?2)
        ON CONFLICT(job_id) DO UPDATE SET
            cancel_requested = excluded.cancel_requested
        "#,
        params![job_id, if cancel_requested { 1 } else { 0 }],
    )?;
    Ok(())
}

pub(crate) fn cancel_requested(conn: &Connection, job_id: &str) -> Result<bool> {
    let requested = conn
        .query_row(
            "SELECT cancel_requested FROM job_details WHERE job_id = ?1",
            [job_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or(0);
    Ok(requested != 0)
}

pub(crate) fn record_interruption(
    conn: &Connection,
    job_id: &str,
    interrupted_at: DateTime<Utc>,
    interruption_reason: &str,
    current_url: Option<&str>,
) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO job_details (
            job_id, job_kind, input_text, output_text, current_url, output_path,
            cancel_requested, interrupted_at, interruption_reason, recovered_at,
            recovery_reason, failed_at, failure_reason
        )
        VALUES (?1, 'ask', NULL, NULL, ?2, NULL, 0, ?3, ?4, NULL, NULL, NULL, NULL)
        ON CONFLICT(job_id) DO UPDATE SET
            current_url = COALESCE(excluded.current_url, job_details.current_url),
            interrupted_at = excluded.interrupted_at,
            interruption_reason = excluded.interruption_reason
        "#,
        params![
            job_id,
            current_url,
            interrupted_at.to_rfc3339(),
            interruption_reason
        ],
    )?;
    Ok(())
}

pub(crate) fn record_recovery(
    conn: &Connection,
    job_id: &str,
    recovered_at: DateTime<Utc>,
    recovery_reason: &str,
    current_url: Option<&str>,
) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO job_details (
            job_id, job_kind, input_text, output_text, current_url, output_path,
            cancel_requested, interrupted_at, interruption_reason, recovered_at,
            recovery_reason, failed_at, failure_reason
        )
        VALUES (?1, 'ask', NULL, NULL, ?2, NULL, 0, NULL, NULL, ?3, ?4, NULL, NULL)
        ON CONFLICT(job_id) DO UPDATE SET
            current_url = COALESCE(excluded.current_url, job_details.current_url),
            recovered_at = excluded.recovered_at,
            recovery_reason = excluded.recovery_reason
        "#,
        params![
            job_id,
            current_url,
            recovered_at.to_rfc3339(),
            recovery_reason
        ],
    )?;
    Ok(())
}

pub(crate) fn record_failure(
    conn: &Connection,
    job_id: &str,
    failed_at: DateTime<Utc>,
    failure_reason: &str,
    current_url: Option<&str>,
) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO job_details (
            job_id, job_kind, input_text, output_text, current_url, output_path,
            cancel_requested, interrupted_at, interruption_reason, recovered_at,
            recovery_reason, failed_at, failure_reason
        )
        VALUES (?1, 'ask', NULL, NULL, ?2, NULL, 0, NULL, NULL, NULL, NULL, ?3, ?4)
        ON CONFLICT(job_id) DO UPDATE SET
            current_url = COALESCE(excluded.current_url, job_details.current_url),
            failed_at = excluded.failed_at,
            failure_reason = excluded.failure_reason
        "#,
        params![job_id, current_url, failed_at.to_rfc3339(), failure_reason],
    )?;
    Ok(())
}

pub(crate) fn update_recovery_metadata(
    conn: &Connection,
    job_id: &str,
    recovery_trace: Option<&RecoveryTrace>,
    recovery_packet: Option<&RecoveryPacket>,
) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO job_details (
            job_id, job_kind, input_text, output_text, current_url, output_path,
            cancel_requested, recovery_trace_json, recovery_packet_json
        )
        VALUES (?1, 'ask', NULL, NULL, NULL, NULL, 0, ?2, ?3)
        ON CONFLICT(job_id) DO UPDATE SET
            recovery_trace_json = COALESCE(
                excluded.recovery_trace_json,
                job_details.recovery_trace_json
            ),
            recovery_packet_json = COALESCE(
                excluded.recovery_packet_json,
                job_details.recovery_packet_json
            )
        "#,
        params![
            job_id,
            recovery_trace.map(serde_json::to_string).transpose()?,
            recovery_packet.map(serde_json::to_string).transpose()?,
        ],
    )?;
    Ok(())
}

pub(crate) fn clear_recovery(conn: &Connection, job_id: &str) -> Result<()> {
    conn.execute(
        r#"
        UPDATE job_details
        SET recovered_at = NULL,
            recovery_reason = NULL,
            recovery_trace_json = NULL,
            recovery_packet_json = NULL
        WHERE job_id = ?1
        "#,
        [job_id],
    )?;
    Ok(())
}

pub(crate) fn detail(conn: &Connection, job_id: &str) -> Result<Option<JobDetail>> {
    let row = conn
        .query_row(
            r#"
            SELECT jobs.job_id,
                   jobs.status,
                   jobs.message,
                   jobs.conversation_id,
                   jobs.created_at,
                   jobs.updated_at,
                   job_details.job_kind,
                   job_details.input_text,
                   job_details.output_text,
                   job_details.current_url,
                   job_details.output_path,
                   job_details.cancel_requested,
                   job_details.interrupted_at,
                   job_details.interruption_reason,
                   job_details.recovered_at,
                   job_details.recovery_reason,
                   job_details.failed_at,
                   job_details.failure_reason,
                   job_details.recovery_trace_json,
                   job_details.recovery_packet_json
            FROM jobs
            LEFT JOIN job_details ON job_details.job_id = jobs.job_id
            WHERE jobs.job_id = ?1
            "#,
            [job_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    parse_job_status(row.get::<_, String>(1)?.as_str()),
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    parse_timestamp(row.get::<_, String>(4)?)?,
                    parse_timestamp(row.get::<_, String>(5)?)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<i64>>(11)?.unwrap_or(0) != 0,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, Option<String>>(16)?,
                    row.get::<_, Option<String>>(17)?,
                    row.get::<_, Option<String>>(18)?,
                    row.get::<_, Option<String>>(19)?,
                ))
            },
        )
        .optional()?;

    let Some((
        job_id,
        status,
        message,
        conversation_id,
        created_at,
        updated_at,
        kind,
        input_text,
        output_text,
        current_url,
        output_path,
        cancel_requested,
        interrupted_at,
        interruption_reason,
        recovered_at,
        recovery_reason,
        failed_at,
        failure_reason,
        recovery_trace_json,
        recovery_packet_json,
    )) = row
    else {
        return Ok(None);
    };

    Ok(Some(build_job_detail(
        job_id.clone(),
        status,
        message,
        conversation_id,
        created_at,
        updated_at,
        kind,
        input_text,
        output_text,
        current_url,
        output_path,
        cancel_requested,
        interrupted_at,
        interruption_reason,
        recovered_at,
        recovery_reason,
        failed_at,
        failure_reason,
        recovery_trace_json,
        recovery_packet_json,
        artifacts::job_artifacts(conn, &job_id)?,
    )?))
}

pub(crate) fn fail_incomplete(
    conn: &Connection,
    message: &str,
    updated_at: DateTime<Utc>,
) -> Result<()> {
    conn.execute(
        r#"
        UPDATE jobs
        SET status = 'failed',
            message = ?1,
            updated_at = ?2
        WHERE status IN (
            'starting', 'running', 'interrupted', 'waiting_for_sources', 'artifact_ready'
        )
        "#,
        params![message, updated_at.to_rfc3339()],
    )?;
    conn.execute(
        r#"
        UPDATE job_details
        SET failed_at = ?1,
            failure_reason = ?2
        WHERE job_id IN (
            SELECT job_id
            FROM jobs
            WHERE status = 'failed' AND updated_at = ?1
        )
        "#,
        params![updated_at.to_rfc3339(), message],
    )?;
    Ok(())
}
