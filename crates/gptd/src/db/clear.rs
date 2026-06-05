use super::DatabaseClearStateResult;
use anyhow::Result;
use rusqlite::{Connection, Transaction};
use shared_protocol::ClearStateResult;

pub(super) fn clear_all_state(conn: &mut Connection) -> Result<DatabaseClearStateResult> {
    let tx = conn.transaction()?;
    let artifact_storage_paths = artifact_storage_paths(&tx)?;
    let result = ClearStateResult {
        sessions_deleted: count_rows(&tx, "sessions")?,
        conversations_deleted: count_rows(&tx, "conversations")?,
        jobs_deleted: count_rows(&tx, "jobs")?,
        job_details_deleted: count_rows(&tx, "job_details")?,
        request_history_deleted: count_rows(&tx, "request_history")?,
        artifacts_deleted: count_rows(&tx, "artifacts")?,
        artifact_links_deleted: count_rows(&tx, "job_artifacts")?,
        failure_bundles_deleted: count_rows(&tx, "failure_bundles")?,
        native_upload_attempts_deleted: count_rows(&tx, "native_upload_attempts")?,
        artifact_files_deleted: 0,
        artifact_files_missing: 0,
        artifact_files_failed: 0,
    };

    tx.execute("DELETE FROM job_artifacts", [])?;
    tx.execute("DELETE FROM request_history", [])?;
    tx.execute("DELETE FROM job_details", [])?;
    tx.execute("DELETE FROM jobs", [])?;
    tx.execute("DELETE FROM artifacts", [])?;
    tx.execute("DELETE FROM conversations", [])?;
    tx.execute("DELETE FROM sessions", [])?;
    tx.execute("DELETE FROM failure_bundles", [])?;
    tx.execute("DELETE FROM native_upload_attempts", [])?;
    tx.commit()?;

    Ok(DatabaseClearStateResult {
        result,
        artifact_storage_paths,
    })
}

fn artifact_storage_paths(tx: &Transaction<'_>) -> Result<Vec<String>> {
    let mut stmt = tx.prepare(
        "SELECT DISTINCT storage_path
         FROM artifacts
         WHERE storage_path IS NOT NULL AND TRIM(storage_path) <> ''
         ORDER BY storage_path ASC",
    )?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    let paths = rows.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(paths)
}

fn count_rows(tx: &Transaction<'_>, table: &str) -> Result<u64> {
    let count = tx.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get::<_, i64>(0)
    })?;
    Ok(count.max(0) as u64)
}
