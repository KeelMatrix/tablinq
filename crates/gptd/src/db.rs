use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use shared_protocol::{
    ArtifactMetadata, JobList, JobStatus, JobSummary, SessionList, SessionSummary,
};
use std::path::Path;

const MIGRATIONS: &[(&str, &str)] = &[
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
];

pub struct Database {
    conn: Connection,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        let db = Self { conn };
        db.apply_migrations()?;
        Ok(db)
    }

    pub fn migration_count(&self) -> Result<u32> {
        let count = self
            .conn
            .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                row.get::<_, u32>(0)
            })
            .context("count schema migrations")?;
        Ok(count)
    }

    pub fn list_sessions(&self) -> Result<SessionList> {
        let mut statement = self.conn.prepare(
            "SELECT session_id, label, tab_url, last_seen_at FROM sessions ORDER BY last_seen_at DESC",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(SessionSummary {
                session_id: row.get(0)?,
                label: row.get(1)?,
                tab_url: row.get(2)?,
                last_seen_at: parse_timestamp(row.get::<_, String>(3)?)?,
            })
        })?;

        let sessions = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(SessionList { sessions })
    }

    pub fn list_jobs(&self) -> Result<JobList> {
        let mut statement = self.conn.prepare(
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

    pub fn upsert_session(
        &self,
        session_id: &str,
        label: &str,
        tab_url: Option<&str>,
        last_seen_at: DateTime<Utc>,
    ) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO sessions (session_id, label, tab_url, last_seen_at)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(session_id) DO UPDATE SET
                label = excluded.label,
                tab_url = excluded.tab_url,
                last_seen_at = excluded.last_seen_at
            "#,
            params![session_id, label, tab_url, last_seen_at.to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn insert_job(
        &self,
        job_id: &str,
        status: JobStatus,
        message: Option<&str>,
        conversation_id: Option<&str>,
        created_at: DateTime<Utc>,
        updated_at: DateTime<Utc>,
    ) -> Result<()> {
        self.conn.execute(
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

    pub fn upsert_conversation(
        &self,
        conversation_id: &str,
        session_id: &str,
        label: &str,
        tab_url: Option<&str>,
        last_prompt: Option<&str>,
        last_response: Option<&str>,
        last_seen_at: DateTime<Utc>,
    ) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO conversations (
                conversation_id, session_id, label, tab_url, last_prompt, last_response, last_seen_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(conversation_id) DO UPDATE SET
                session_id = excluded.session_id,
                label = excluded.label,
                tab_url = excluded.tab_url,
                last_prompt = excluded.last_prompt,
                last_response = excluded.last_response,
                last_seen_at = excluded.last_seen_at
            "#,
            params![
                conversation_id,
                session_id,
                label,
                tab_url,
                last_prompt,
                last_response,
                last_seen_at.to_rfc3339()
            ],
        )?;
        Ok(())
    }

    pub fn latest_conversation_id(&self) -> Result<Option<String>> {
        let conversation_id = self
            .conn
            .query_row(
                "SELECT conversation_id FROM conversations ORDER BY last_seen_at DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(conversation_id)
    }

    pub fn insert_artifact(
        &self,
        artifact: &ArtifactMetadata,
        created_at: DateTime<Utc>,
    ) -> Result<()> {
        self.conn.execute(
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
                artifact.size_bytes,
                artifact.sha256,
                artifact.storage_path,
                created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    fn apply_migrations(&self) -> Result<()> {
        self.conn.execute_batch(
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
            let applied = self
                .conn
                .query_row(
                    "SELECT applied_at FROM schema_migrations WHERE name = ?1",
                    [name],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            if applied.is_none() {
                self.conn.execute_batch(sql)?;
                self.conn.execute(
                    "INSERT INTO schema_migrations (name, applied_at) VALUES (?1, ?2)",
                    params![name, Utc::now().to_rfc3339()],
                )?;
            }
        }
        Ok(())
    }
}

fn parse_timestamp(value: String) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(err))
        })
}

fn parse_job_status(value: &str) -> JobStatus {
    match value {
        "queued" => JobStatus::Queued,
        "starting" => JobStatus::Starting,
        "running" => JobStatus::Running,
        "waiting_for_sources" => JobStatus::WaitingForSources,
        "completed" => JobStatus::Completed,
        "artifact_ready" => JobStatus::ArtifactReady,
        _ => JobStatus::Failed,
    }
}

fn job_status_as_str(value: JobStatus) -> &'static str {
    match value {
        JobStatus::Queued => "queued",
        JobStatus::Starting => "starting",
        JobStatus::Running => "running",
        JobStatus::WaitingForSources => "waiting_for_sources",
        JobStatus::Completed => "completed",
        JobStatus::Failed => "failed",
        JobStatus::ArtifactReady => "artifact_ready",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn migrations_are_applied_on_open() {
        let dir = tempdir().unwrap();
        let db = Database::open(&dir.path().join("tablinq.sqlite3")).unwrap();
        assert_eq!(db.migration_count().unwrap(), MIGRATIONS.len() as u32);
    }
}
