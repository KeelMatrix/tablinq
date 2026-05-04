use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use shared_protocol::{ArtifactList, ArtifactMetadata, ArtifactSummary};
use std::fs;

use super::row_mapping::map_artifact_row;

pub(crate) fn insert(
    conn: &Connection,
    artifact: &ArtifactMetadata,
    created_at: DateTime<Utc>,
) -> Result<()> {
    let size_bytes =
        i64::try_from(artifact.size_bytes).context("artifact size exceeds sqlite INTEGER range")?;
    conn.execute(
        r#"
        INSERT OR REPLACE INTO artifacts (
            artifact_id, conversation_id, message_id, file_name, content_type, size_bytes,
            sha256, storage_path, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        params![
            artifact.artifact_id,
            artifact.conversation_id,
            artifact.message_id,
            artifact.file_name,
            artifact.content_type,
            size_bytes,
            artifact.sha256,
            artifact.storage_path,
            created_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub(crate) fn prune(conn: &Connection, limit: usize) -> Result<usize> {
    let stale = {
        let mut statement = conn.prepare(
            r#"
            SELECT artifact_id, storage_path
            FROM artifacts
            WHERE artifact_id NOT IN (
                SELECT artifact_id
                FROM artifacts
                ORDER BY created_at DESC
                LIMIT ?1
            )
            "#,
        )?;
        let rows = statement.query_map([limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };

    for (artifact_id, storage_path) in &stale {
        let _ = fs::remove_file(storage_path);
        conn.execute(
            "DELETE FROM job_artifacts WHERE artifact_id = ?1",
            [artifact_id.as_str()],
        )?;
        conn.execute(
            "DELETE FROM artifacts WHERE artifact_id = ?1",
            [artifact_id.as_str()],
        )?;
    }

    Ok(stale.len())
}

pub(crate) fn link_job(conn: &Connection, job_id: &str, artifact_id: &str) -> Result<()> {
    conn.execute(
        r#"
        INSERT OR IGNORE INTO job_artifacts (job_id, artifact_id)
        VALUES (?1, ?2)
        "#,
        params![job_id, artifact_id],
    )?;
    Ok(())
}

pub(crate) fn list(conn: &Connection) -> Result<ArtifactList> {
    let mut statement = conn.prepare(
        r#"
        SELECT artifact_id, conversation_id, message_id, file_name, content_type,
               size_bytes, sha256, storage_path, created_at
        FROM artifacts
        ORDER BY created_at DESC
        "#,
    )?;
    let rows = statement.query_map([], map_artifact_row)?;
    let artifacts = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(ArtifactList { artifacts })
}

pub(crate) fn latest(conn: &Connection) -> Result<Option<ArtifactSummary>> {
    let artifact = conn
        .query_row(
            r#"
            SELECT artifact_id, conversation_id, message_id, file_name, content_type,
                   size_bytes, sha256, storage_path, created_at
            FROM artifacts
            ORDER BY created_at DESC
            LIMIT 1
            "#,
            [],
            map_artifact_row,
        )
        .optional()?;
    Ok(artifact)
}

pub(crate) fn latest_request_by_content_type(
    conn: &Connection,
    request_id: &str,
    content_type: &str,
) -> Result<Option<ArtifactSummary>> {
    conn.query_row(
        r#"
        SELECT artifacts.artifact_id,
               artifacts.conversation_id,
               artifacts.message_id,
               artifacts.file_name,
               artifacts.content_type,
               artifacts.size_bytes,
               artifacts.sha256,
               artifacts.storage_path,
               artifacts.created_at
        FROM request_history
        INNER JOIN job_artifacts ON job_artifacts.job_id = request_history.job_id
        INNER JOIN artifacts ON artifacts.artifact_id = job_artifacts.artifact_id
        WHERE request_history.request_id = ?1
          AND artifacts.content_type = ?2
        ORDER BY artifacts.created_at DESC
        LIMIT 1
        "#,
        params![request_id, content_type],
        map_artifact_row,
    )
    .optional()
    .map_err(Into::into)
}

pub(crate) fn find(conn: &Connection, artifact_id: &str) -> Result<Option<ArtifactSummary>> {
    let artifact = conn
        .query_row(
            r#"
            SELECT artifact_id, conversation_id, message_id, file_name, content_type,
                   size_bytes, sha256, storage_path, created_at
            FROM artifacts
            WHERE artifact_id = ?1
            "#,
            [artifact_id],
            map_artifact_row,
        )
        .optional()?;
    Ok(artifact)
}

pub(crate) fn job_ids(conn: &Connection, artifact_id: &str) -> Result<Vec<String>> {
    let mut statement = conn.prepare(
        r#"
        SELECT job_id
        FROM job_artifacts
        WHERE artifact_id = ?1
        ORDER BY job_id ASC
        "#,
    )?;
    let rows = statement.query_map([artifact_id], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub(crate) fn request_ids(conn: &Connection, artifact_id: &str) -> Result<Vec<String>> {
    let mut statement = conn.prepare(
        r#"
        SELECT DISTINCT request_history.request_id
        FROM job_artifacts
        INNER JOIN request_history ON request_history.job_id = job_artifacts.job_id
        WHERE job_artifacts.artifact_id = ?1
        ORDER BY request_history.request_id ASC
        "#,
    )?;
    let rows = statement.query_map([artifact_id], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub(crate) fn job_artifacts(conn: &Connection, job_id: &str) -> Result<Vec<ArtifactSummary>> {
    let mut statement = conn.prepare(
        r#"
        SELECT artifacts.artifact_id,
               artifacts.conversation_id,
               artifacts.message_id,
               artifacts.file_name,
               artifacts.content_type,
               artifacts.size_bytes,
               artifacts.sha256,
               artifacts.storage_path,
               artifacts.created_at
        FROM job_artifacts
        INNER JOIN artifacts ON artifacts.artifact_id = job_artifacts.artifact_id
        WHERE job_artifacts.job_id = ?1
        ORDER BY artifacts.created_at DESC
        "#,
    )?;
    let rows = statement.query_map([job_id], map_artifact_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}
