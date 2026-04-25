use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};

pub(crate) const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_initial",
        r#"
        CREATE TABLE IF NOT EXISTS schema_migrations (
            name TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS sessions (
            session_id TEXT PRIMARY KEY,
            label TEXT NOT NULL,
            tab_url TEXT,
            last_seen_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS jobs (
            job_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            message TEXT,
            conversation_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS artifacts (
            artifact_id TEXT PRIMARY KEY,
            conversation_id TEXT,
            message_id TEXT,
            file_name TEXT NOT NULL,
            content_type TEXT NOT NULL,
            size_bytes INTEGER NOT NULL,
            sha256 TEXT NOT NULL,
            storage_path TEXT NOT NULL,
            created_at TEXT NOT NULL
        );
        "#,
    ),
    (
        "0002_indexes",
        r#"
        CREATE INDEX IF NOT EXISTS idx_sessions_last_seen_at ON sessions(last_seen_at DESC);
        CREATE INDEX IF NOT EXISTS idx_jobs_updated_at ON jobs(updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_artifacts_created_at ON artifacts(created_at DESC);
        "#,
    ),
    (
        "0003_conversations",
        r#"
        CREATE TABLE IF NOT EXISTS conversations (
            conversation_id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            label TEXT NOT NULL,
            tab_url TEXT,
            last_prompt TEXT,
            last_response TEXT,
            last_seen_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_conversations_last_seen_at ON conversations(last_seen_at DESC);
        "#,
    ),
    (
        "0004_job_details",
        r#"
        CREATE TABLE IF NOT EXISTS job_details (
            job_id TEXT PRIMARY KEY,
            job_kind TEXT NOT NULL,
            input_text TEXT,
            output_text TEXT,
            current_url TEXT,
            output_path TEXT,
            cancel_requested INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS job_artifacts (
            job_id TEXT NOT NULL,
            artifact_id TEXT NOT NULL,
            PRIMARY KEY (job_id, artifact_id)
        );

        CREATE INDEX IF NOT EXISTS idx_job_artifacts_job_id ON job_artifacts(job_id);
        "#,
    ),
    (
        "0005_failure_bundles",
        r#"
        CREATE TABLE IF NOT EXISTS failure_bundles (
            bundle_id TEXT PRIMARY KEY,
            created_at TEXT NOT NULL,
            payload_json TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_failure_bundles_created_at
            ON failure_bundles(created_at DESC);
        "#,
    ),
    (
        "0006_request_history",
        r#"
        CREATE TABLE IF NOT EXISTS request_history (
            request_id TEXT PRIMARY KEY,
            job_id TEXT NOT NULL,
            model_id TEXT,
            request_role TEXT,
            parent_request_id TEXT,
            execution_kind TEXT NOT NULL,
            status TEXT NOT NULL,
            cancelled INTEGER NOT NULL DEFAULT 0,
            start_time TEXT NOT NULL,
            end_time TEXT,
            duration_ms INTEGER,
            queue_wait_ms INTEGER NOT NULL DEFAULT 0,
            usage_status TEXT NOT NULL DEFAULT 'unknown',
            prompt_tokens INTEGER,
            completion_tokens INTEGER,
            total_tokens INTEGER,
            warnings_json TEXT NOT NULL DEFAULT '[]'
        );

        CREATE INDEX IF NOT EXISTS idx_request_history_start_time
            ON request_history(start_time DESC);
        "#,
    ),
    (
        "0007_request_recovery",
        r#"
        ALTER TABLE request_history ADD COLUMN interrupted_at TEXT;
        ALTER TABLE request_history ADD COLUMN interruption_reason TEXT;
        ALTER TABLE request_history ADD COLUMN recovered_at TEXT;
        ALTER TABLE request_history ADD COLUMN recovery_reason TEXT;
        ALTER TABLE request_history ADD COLUMN failed_at TEXT;
        ALTER TABLE request_history ADD COLUMN failure_reason TEXT;

        ALTER TABLE job_details ADD COLUMN interrupted_at TEXT;
        ALTER TABLE job_details ADD COLUMN interruption_reason TEXT;
        ALTER TABLE job_details ADD COLUMN recovered_at TEXT;
        ALTER TABLE job_details ADD COLUMN recovery_reason TEXT;
        ALTER TABLE job_details ADD COLUMN failed_at TEXT;
        ALTER TABLE job_details ADD COLUMN failure_reason TEXT;
        "#,
    ),
    (
        "0008_request_recovery_metadata",
        r#"
        ALTER TABLE request_history ADD COLUMN recovery_trace_json TEXT;
        ALTER TABLE request_history ADD COLUMN recovery_packet_json TEXT;

        ALTER TABLE job_details ADD COLUMN recovery_trace_json TEXT;
        ALTER TABLE job_details ADD COLUMN recovery_packet_json TEXT;
        "#,
    ),
    (
        "0009_conversation_recovery_manifest",
        r#"
        ALTER TABLE conversations ADD COLUMN recovery_manifest_json TEXT;
        "#,
    ),
];

pub(crate) fn apply(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS schema_migrations (
            name TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL
        );
        "#,
    )?;
    for (name, sql) in MIGRATIONS {
        let applied = conn
            .query_row(
                "SELECT applied_at FROM schema_migrations WHERE name = ?1",
                [name],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if applied.is_none() {
            conn.execute_batch(sql)?;
            conn.execute(
                "INSERT INTO schema_migrations (name, applied_at) VALUES (?1, ?2)",
                params![name, Utc::now().to_rfc3339()],
            )?;
        }
    }
    Ok(())
}

pub(crate) fn count(conn: &Connection) -> Result<u32> {
    let count = conn
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get::<_, u32>(0)
        })
        .context("count schema migrations")?;
    Ok(count)
}
