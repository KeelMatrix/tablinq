use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use shared_protocol::{
    ArtifactSummary, ConversationList, ConversationRecoveryManifest, ConversationSummary,
};

use super::{
    ConversationTurnRecord, artifacts,
    row_mapping::{
        build_job_detail, map_artifact_row, map_conversation_row, map_optional_request_summary_row,
        parse_job_status, parse_timestamp,
    },
};

pub(crate) fn find(
    conn: &Connection,
    conversation_id: &str,
) -> Result<Option<ConversationSummary>> {
    conn.query_row(
        r#"
        SELECT conversation_id, session_id, label, tab_url, last_prompt, last_response,
               last_seen_at
        FROM conversations
        WHERE conversation_id = ?1
        "#,
        [conversation_id],
        map_conversation_row,
    )
    .optional()
    .map_err(Into::into)
}

pub(crate) fn recovery_manifest(
    conn: &Connection,
    conversation_id: &str,
) -> Result<Option<ConversationRecoveryManifest>> {
    conn.query_row(
        r#"
        SELECT recovery_manifest_json
        FROM conversations
        WHERE conversation_id = ?1
        "#,
        [conversation_id],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()
    .map_err(anyhow::Error::from)
    .and_then(|value| match value.flatten() {
        Some(json) => Ok(Some(serde_json::from_str(&json)?)),
        None => Ok(None),
    })
}

pub(crate) fn list(conn: &Connection) -> Result<ConversationList> {
    let mut statement = conn.prepare(
        r#"
        SELECT conversation_id, session_id, label, tab_url, last_prompt, last_response,
               last_seen_at
        FROM conversations
        ORDER BY last_seen_at DESC
        "#,
    )?;
    let rows = statement.query_map([], map_conversation_row)?;
    let conversations = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(ConversationList { conversations })
}

pub(crate) fn list_for_session(
    conn: &Connection,
    session_id: &str,
) -> Result<Vec<ConversationSummary>> {
    let mut statement = conn.prepare(
        r#"
        SELECT conversation_id, session_id, label, tab_url, last_prompt, last_response,
               last_seen_at
        FROM conversations
        WHERE session_id = ?1
        ORDER BY last_seen_at DESC
        "#,
    )?;
    let rows = statement.query_map([session_id], map_conversation_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub(crate) fn list_turn_records(
    conn: &Connection,
    conversation_id: &str,
) -> Result<Vec<ConversationTurnRecord>> {
    let mut statement = conn.prepare(
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
               job_details.recovery_packet_json,
               request_history.request_id,
               request_history.job_id,
               request_history.model_id,
               request_history.request_role,
               request_history.parent_request_id,
               request_history.execution_kind,
               request_history.status,
               request_history.cancelled,
               request_history.start_time,
               request_history.end_time,
               request_history.duration_ms,
               request_history.queue_wait_ms,
               request_history.interrupted_at,
               request_history.interruption_reason,
               request_history.recovered_at,
               request_history.recovery_reason,
               request_history.failed_at,
               request_history.failure_reason,
               request_history.recovery_trace_json,
               request_history.recovery_packet_json,
               request_history.usage_status,
               request_history.prompt_tokens,
               request_history.completion_tokens,
               request_history.total_tokens,
               request_history.warnings_json
        FROM jobs
        LEFT JOIN job_details ON job_details.job_id = jobs.job_id
        LEFT JOIN request_history ON request_history.job_id = jobs.job_id
        WHERE jobs.conversation_id = ?1
        ORDER BY jobs.created_at ASC, jobs.job_id ASC
        "#,
    )?;
    let rows = statement.query_map([conversation_id], |row| {
        let job_id = row.get::<_, String>(0)?;
        let status = parse_job_status(row.get::<_, String>(1)?.as_str());
        let message = row.get::<_, Option<String>>(2)?;
        let conversation_id = row.get::<_, Option<String>>(3)?;
        let created_at = parse_timestamp(row.get::<_, String>(4)?)?;
        let updated_at = parse_timestamp(row.get::<_, String>(5)?)?;
        let kind = row.get::<_, Option<String>>(6)?;
        let input_text = row.get::<_, Option<String>>(7)?;
        let output_text = row.get::<_, Option<String>>(8)?;
        let current_url = row.get::<_, Option<String>>(9)?;
        let output_path = row.get::<_, Option<String>>(10)?;
        let cancel_requested = row.get::<_, Option<i64>>(11)?.unwrap_or(0) != 0;
        let interrupted_at = row.get::<_, Option<String>>(12)?;
        let interruption_reason = row.get::<_, Option<String>>(13)?;
        let recovered_at = row.get::<_, Option<String>>(14)?;
        let recovery_reason = row.get::<_, Option<String>>(15)?;
        let failed_at = row.get::<_, Option<String>>(16)?;
        let failure_reason = row.get::<_, Option<String>>(17)?;
        let recovery_trace_json = row.get::<_, Option<String>>(18)?;
        let recovery_packet_json = row.get::<_, Option<String>>(19)?;
        let request = map_optional_request_summary_row(row, 20)?;
        Ok((
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
            request,
        ))
    })?;

    let mut records = Vec::new();
    for row in rows {
        let (
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
            request,
        ) = row?;
        let job_artifacts = artifacts::job_artifacts(conn, &job_id)?;
        records.push(ConversationTurnRecord {
            job: build_job_detail(
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
                job_artifacts,
            )?,
            request,
        });
    }
    Ok(records)
}

pub(crate) fn list_artifacts(
    conn: &Connection,
    conversation_id: &str,
) -> Result<Vec<ArtifactSummary>> {
    let mut statement = conn.prepare(
        r#"
        SELECT artifact_id,
               conversation_id,
               message_id,
               file_name,
               content_type,
               size_bytes,
               sha256,
               storage_path,
               created_at
        FROM artifacts
        WHERE conversation_id = ?1
        ORDER BY created_at DESC, artifact_id DESC
        "#,
    )?;
    let rows = statement.query_map([conversation_id], map_artifact_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn upsert(
    conn: &Connection,
    conversation_id: &str,
    session_id: &str,
    label: &str,
    tab_url: Option<&str>,
    last_prompt: Option<&str>,
    last_response: Option<&str>,
    recovery_manifest: Option<&ConversationRecoveryManifest>,
    last_seen_at: DateTime<Utc>,
) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO conversations (
            conversation_id, session_id, label, tab_url, last_prompt, last_response,
            recovery_manifest_json, last_seen_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT(conversation_id) DO UPDATE SET
            session_id = excluded.session_id,
            label = excluded.label,
            tab_url = excluded.tab_url,
            last_prompt = excluded.last_prompt,
            last_response = excluded.last_response,
            recovery_manifest_json = excluded.recovery_manifest_json,
            last_seen_at = excluded.last_seen_at
        "#,
        params![
            conversation_id,
            session_id,
            label,
            tab_url,
            last_prompt,
            last_response,
            recovery_manifest.map(serde_json::to_string).transpose()?,
            last_seen_at.to_rfc3339()
        ],
    )?;
    Ok(())
}

pub(crate) fn latest_id(conn: &Connection) -> Result<Option<String>> {
    let conversation_id = conn
        .query_row(
            "SELECT conversation_id FROM conversations ORDER BY last_seen_at DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(conversation_id)
}

pub(crate) fn recent_for_session(
    conn: &Connection,
    session_id: &str,
    limit: usize,
) -> Result<Vec<ConversationSummary>> {
    let mut statement = conn.prepare(
        r#"
        SELECT conversation_id, session_id, label, tab_url, last_prompt, last_response,
               last_seen_at
        FROM conversations
        WHERE session_id = ?1
        ORDER BY last_seen_at DESC
        LIMIT ?2
        "#,
    )?;
    let rows = statement.query_map(params![session_id, limit as i64], map_conversation_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}
