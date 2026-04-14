use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use shared_protocol::{
    ArtifactList, ArtifactMetadata, ArtifactSummary, ConversationSummary, FailureBundle, JobDetail,
    JobKind, JobList, JobStatus, JobSummary, RequestExecutionKind, RequestHistory, RequestSummary,
    RequestUsage, SessionList, SessionSummary,
};
use std::fs;
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
];

const REQUEST_HISTORY_LIMIT: usize = 200;
#[cfg(test)]
const ARTIFACT_HISTORY_LIMIT: usize = 100;

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
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                parse_timestamp(row.get::<_, String>(3)?)?,
            ))
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            let (session_id, label, tab_url, last_seen_at) = row?;
            let recent_conversations = self.recent_conversations_for_session(&session_id, 10)?;
            let latest_conversation_id = recent_conversations
                .first()
                .map(|conversation| conversation.conversation_id.clone());
            sessions.push(SessionSummary {
                session_id,
                label,
                tab_url,
                last_seen_at,
                latest_conversation_id,
                recent_conversations,
            });
        }
        Ok(SessionList { sessions })
    }

    pub fn find_conversation(&self, conversation_id: &str) -> Result<Option<ConversationSummary>> {
        self.conn
            .query_row(
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

    pub fn upsert_request_summary(&self, summary: &RequestSummary) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO request_history (
                request_id, job_id, model_id, request_role, parent_request_id, execution_kind,
                status, cancelled, start_time, end_time, duration_ms, queue_wait_ms,
                interrupted_at, interruption_reason, recovered_at, recovery_reason, failed_at,
                failure_reason, usage_status, prompt_tokens, completion_tokens, total_tokens,
                warnings_json
            )
            VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
                ?19, ?20, ?21, ?22, ?23
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
                summary.usage.status,
                summary.usage.prompt_tokens,
                summary.usage.completion_tokens,
                summary.usage.total_tokens,
                serde_json::to_string(&summary.warnings)?,
            ],
        )?;
        self.prune_request_history(REQUEST_HISTORY_LIMIT)?;
        Ok(())
    }

    pub fn request_summary(&self, request_id: &str) -> Result<Option<RequestSummary>> {
        self.conn
            .query_row(
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

    pub fn list_request_summaries(&self, limit: usize) -> Result<RequestHistory> {
        let mut statement = self.conn.prepare(
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

    pub fn upsert_job_detail(
        &self,
        job_id: &str,
        kind: JobKind,
        input_text: Option<&str>,
        output_path: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
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

    pub fn update_job_output(
        &self,
        job_id: &str,
        output_text: Option<&str>,
        current_url: Option<&str>,
        output_path: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
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

    pub fn set_job_cancel_requested(&self, job_id: &str, cancel_requested: bool) -> Result<()> {
        self.conn.execute(
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

    pub fn job_cancel_requested(&self, job_id: &str) -> Result<bool> {
        let requested = self
            .conn
            .query_row(
                "SELECT cancel_requested FROM job_details WHERE job_id = ?1",
                [job_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0);
        Ok(requested != 0)
    }

    pub fn record_job_interruption(
        &self,
        job_id: &str,
        interrupted_at: DateTime<Utc>,
        interruption_reason: &str,
        current_url: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
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

    pub fn record_job_recovery(
        &self,
        job_id: &str,
        recovered_at: DateTime<Utc>,
        recovery_reason: &str,
        current_url: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
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

    pub fn record_job_failure(
        &self,
        job_id: &str,
        failed_at: DateTime<Utc>,
        failure_reason: &str,
        current_url: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
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

    #[allow(clippy::too_many_arguments)]
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

    fn recent_conversations_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<ConversationSummary>> {
        let mut statement = self.conn.prepare(
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

    pub fn prune_artifacts(&self, limit: usize) -> Result<usize> {
        let stale = {
            let mut statement = self.conn.prepare(
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
            self.conn.execute(
                "DELETE FROM job_artifacts WHERE artifact_id = ?1",
                [artifact_id.as_str()],
            )?;
            self.conn.execute(
                "DELETE FROM artifacts WHERE artifact_id = ?1",
                [artifact_id.as_str()],
            )?;
        }

        Ok(stale.len())
    }

    pub fn link_job_artifact(&self, job_id: &str, artifact_id: &str) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT OR IGNORE INTO job_artifacts (job_id, artifact_id)
            VALUES (?1, ?2)
            "#,
            params![job_id, artifact_id],
        )?;
        Ok(())
    }

    pub fn list_artifacts(&self) -> Result<ArtifactList> {
        let mut statement = self.conn.prepare(
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

    pub fn latest_artifact(&self) -> Result<Option<ArtifactSummary>> {
        let artifact = self
            .conn
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

    pub fn find_artifact(&self, artifact_id: &str) -> Result<Option<ArtifactSummary>> {
        let artifact = self
            .conn
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

    pub fn artifact_job_ids(&self, artifact_id: &str) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare(
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

    pub fn artifact_request_ids(&self, artifact_id: &str) -> Result<Vec<String>> {
        let mut statement = self.conn.prepare(
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

    pub fn job_detail(&self, job_id: &str) -> Result<Option<JobDetail>> {
        let row = self
            .conn
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
                       job_details.failure_reason
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
        )) = row
        else {
            return Ok(None);
        };

        Ok(Some(JobDetail {
            job_id: job_id.clone(),
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
            artifacts: self.job_artifacts(&job_id)?,
            cancel_requested,
            interrupted_at: interrupted_at.map(parse_timestamp).transpose()?,
            interruption_reason,
            recovered_at: recovered_at.map(parse_timestamp).transpose()?,
            recovery_reason,
            failed_at: failed_at.map(parse_timestamp).transpose()?,
            failure_reason,
        }))
    }

    pub fn fail_incomplete_jobs(&self, message: &str, updated_at: DateTime<Utc>) -> Result<()> {
        self.conn.execute(
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
        self.conn.execute(
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

    pub fn insert_failure_bundle(&self, bundle: &FailureBundle) -> Result<()> {
        self.conn.execute(
            r#"
            INSERT OR REPLACE INTO failure_bundles (bundle_id, created_at, payload_json)
            VALUES (?1, ?2, ?3)
            "#,
            params![
                bundle.bundle_id,
                bundle.created_at.to_rfc3339(),
                serde_json::to_string(bundle)?,
            ],
        )?;
        Ok(())
    }

    pub fn count_failure_bundles(&self) -> Result<u64> {
        let count = self
            .conn
            .query_row("SELECT COUNT(*) FROM failure_bundles", [], |row| {
                row.get::<_, u64>(0)
            })?;
        Ok(count)
    }

    pub fn latest_failure_bundle(&self) -> Result<Option<FailureBundle>> {
        let payload = self
            .conn
            .query_row(
                "SELECT payload_json FROM failure_bundles ORDER BY created_at DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        payload
            .map(|raw| serde_json::from_str(&raw).context("decode failure bundle from sqlite"))
            .transpose()
    }

    fn job_artifacts(&self, job_id: &str) -> Result<Vec<ArtifactSummary>> {
        let mut statement = self.conn.prepare(
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

    fn prune_request_history(&self, limit: usize) -> Result<()> {
        self.conn.execute(
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

fn map_conversation_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ConversationSummary> {
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

fn map_artifact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArtifactSummary> {
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

fn map_request_summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RequestSummary> {
    let warnings_json = row.get::<_, String>(22)?;
    let warnings = serde_json::from_str(&warnings_json).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(22, rusqlite::types::Type::Text, Box::new(err))
    })?;
    Ok(RequestSummary {
        request_id: row.get(0)?,
        job_id: row.get(1)?,
        model_id: row.get(2)?,
        request_role: row.get(3)?,
        parent_request_id: row.get(4)?,
        execution_kind: parse_request_execution_kind(row.get::<_, String>(5)?.as_str()),
        status: parse_job_status(row.get::<_, String>(6)?.as_str()),
        cancelled: row.get::<_, i64>(7)? != 0,
        start_time: parse_timestamp(row.get::<_, String>(8)?)?,
        end_time: row
            .get::<_, Option<String>>(9)?
            .map(parse_timestamp)
            .transpose()?,
        duration_ms: row.get(10)?,
        queue_wait_ms: row.get::<_, i64>(11)?.max(0) as u64,
        interrupted_at: row
            .get::<_, Option<String>>(12)?
            .map(parse_timestamp)
            .transpose()?,
        interruption_reason: row.get(13)?,
        recovered_at: row
            .get::<_, Option<String>>(14)?
            .map(parse_timestamp)
            .transpose()?,
        recovery_reason: row.get(15)?,
        failed_at: row
            .get::<_, Option<String>>(16)?
            .map(parse_timestamp)
            .transpose()?,
        failure_reason: row.get(17)?,
        usage: RequestUsage {
            status: row.get(18)?,
            prompt_tokens: row.get(19)?,
            completion_tokens: row.get(20)?,
            total_tokens: row.get(21)?,
        },
        warnings,
    })
}

fn parse_job_status(value: &str) -> JobStatus {
    match value {
        "queued" => JobStatus::Queued,
        "starting" => JobStatus::Starting,
        "running" => JobStatus::Running,
        "interrupted" => JobStatus::Interrupted,
        "recovered" => JobStatus::Recovered,
        "waiting_for_sources" => JobStatus::WaitingForSources,
        "completed" => JobStatus::Completed,
        "cancelled" => JobStatus::Cancelled,
        "artifact_ready" => JobStatus::ArtifactReady,
        _ => JobStatus::Failed,
    }
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

fn job_status_as_str(value: JobStatus) -> &'static str {
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

fn job_kind_as_str(value: JobKind) -> &'static str {
    match value {
        JobKind::Ask => "ask",
        JobKind::Research => "research",
    }
}

fn request_execution_kind_as_str(value: RequestExecutionKind) -> &'static str {
    match value {
        RequestExecutionKind::Ask => "ask",
        RequestExecutionKind::Research => "research",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use tempfile::tempdir;

    #[test]
    fn migrations_are_applied_on_open() {
        let dir = tempdir().unwrap();
        let db = Database::open(&dir.path().join("tablinq.sqlite3")).unwrap();
        assert_eq!(db.migration_count().unwrap(), MIGRATIONS.len() as u32);
    }

    #[test]
    fn request_history_is_retained_with_a_fixed_bound() {
        let dir = tempdir().unwrap();
        let db = Database::open(&dir.path().join("tablinq.sqlite3")).unwrap();
        let start = Utc::now();

        for index in 0..(REQUEST_HISTORY_LIMIT as u64 + 5) {
            let timestamp = start + Duration::milliseconds(index as i64);
            db.upsert_request_summary(&RequestSummary {
                request_id: format!("req-{index}"),
                job_id: format!("job-{index}"),
                model_id: Some("tablinq-instant".to_owned()),
                request_role: Some("primary".to_owned()),
                parent_request_id: None,
                execution_kind: RequestExecutionKind::Ask,
                status: JobStatus::Completed,
                cancelled: false,
                start_time: timestamp,
                end_time: Some(timestamp),
                duration_ms: Some(1),
                queue_wait_ms: 0,
                interrupted_at: None,
                interruption_reason: None,
                recovered_at: None,
                recovery_reason: None,
                failed_at: None,
                failure_reason: None,
                usage: RequestUsage {
                    status: "unknown".to_owned(),
                    prompt_tokens: None,
                    completion_tokens: None,
                    total_tokens: None,
                },
                warnings: vec![
                    "exact token usage is unavailable for the browser-backed provider".to_owned(),
                ],
            })
            .unwrap();
        }

        let history = db
            .list_request_summaries(REQUEST_HISTORY_LIMIT + 10)
            .unwrap();
        assert_eq!(history.requests.len(), REQUEST_HISTORY_LIMIT);
        assert_eq!(
            history.requests[0].request_id,
            format!("req-{}", REQUEST_HISTORY_LIMIT as u64 + 4)
        );
        assert_eq!(history.requests.last().unwrap().request_id, "req-5");
        assert!(db.request_summary("req-0").unwrap().is_none());
    }

    #[test]
    fn artifacts_are_pruned_with_a_fixed_bound() {
        let dir = tempdir().unwrap();
        let db = Database::open(&dir.path().join("tablinq.sqlite3")).unwrap();
        let now = Utc::now();

        for index in 0..(ARTIFACT_HISTORY_LIMIT as u64 + 2) {
            let storage_path = dir.path().join(format!("artifact-{index}.txt"));
            fs::write(&storage_path, format!("artifact-{index}")).unwrap();
            db.insert_artifact(
                &ArtifactMetadata {
                    artifact_id: format!("artifact-{index}"),
                    conversation_id: Some("conv-1".to_owned()),
                    message_id: Some(format!("msg-{index}")),
                    file_name: format!("artifact-{index}.txt"),
                    content_type: "text/plain".to_owned(),
                    size_bytes: 10,
                    sha256: format!("sha-{index}"),
                    storage_path: storage_path.display().to_string(),
                },
                now + Duration::milliseconds(index as i64),
            )
            .unwrap();
        }

        let removed = db.prune_artifacts(ARTIFACT_HISTORY_LIMIT).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(
            db.list_artifacts().unwrap().artifacts.len(),
            ARTIFACT_HISTORY_LIMIT
        );
        assert!(db.find_artifact("artifact-0").unwrap().is_none());
        assert!(!dir.path().join("artifact-0.txt").exists());
    }

    #[test]
    fn conversation_summaries_are_listed_and_persisted() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("tablinq.sqlite3");
        let now = Utc::now();

        {
            let db = Database::open(&path).unwrap();
            db.upsert_session(
                "browser-primary",
                "Managed browser",
                Some("http://example.test"),
                now,
            )
            .unwrap();
            db.upsert_conversation(
                "conv-1",
                "browser-primary",
                "Managed conversation",
                Some("http://example.test/?conversation=conv-1"),
                Some("first prompt"),
                Some("first response"),
                now,
            )
            .unwrap();
            db.upsert_conversation(
                "conv-2",
                "browser-primary",
                "Managed conversation",
                Some("http://example.test/?conversation=conv-2"),
                Some("second prompt"),
                Some("second response"),
                now + Duration::seconds(1),
            )
            .unwrap();
        }

        let db = Database::open(&path).unwrap();
        let conversation = db.find_conversation("conv-1").unwrap().unwrap();
        assert_eq!(conversation.session_id, "browser-primary");
        assert_eq!(conversation.last_prompt.as_deref(), Some("first prompt"));

        let sessions = db.list_sessions().unwrap();
        assert_eq!(sessions.sessions.len(), 1);
        assert_eq!(
            sessions.sessions[0].latest_conversation_id.as_deref(),
            Some("conv-2")
        );
        assert_eq!(sessions.sessions[0].recent_conversations.len(), 2);
        assert_eq!(
            sessions.sessions[0].recent_conversations[0].conversation_id,
            "conv-2"
        );
    }
}
