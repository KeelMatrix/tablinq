use super::recovery_manifest::update_conversation_recovery_manifest;
use super::support::{
    browser_session_identity, is_cancelled_message, lock_browser, lock_cancel_registry,
};
use super::{ARTIFACT_RETENTION_LIMIT, CoreService};
use crate::config::AppConfig;
use crate::db::Database;
use anyhow::Result;
use chrono::{DateTime, Utc};
use shared_protocol::{
    ArtifactMetadata, ConversationRecoveryManifest, EventPayload, JobKind, JobState, JobStatus,
    RecoveryPacket, RecoveryTrace,
};

#[derive(Debug, Clone)]
pub(super) struct PreparedJobRecord {
    pub job_id: String,
    pub conversation_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct JobLifecycle {
    pub kind: JobKind,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct JobLifecycleRef<'a> {
    pub job_id: &'a str,
    pub created_at: DateTime<Utc>,
    pub conversation_id: Option<&'a str>,
    pub lifecycle: JobLifecycle,
}

pub(super) struct PrepareJobInput<'a> {
    pub kind: JobKind,
    pub input_text: &'a str,
    pub conversation_id: Option<String>,
    pub new_conversation: bool,
}

pub(super) struct ConversationFinalization<'a> {
    pub conversation_id: &'a str,
    pub current_url: &'a str,
    pub prompt: &'a str,
    pub output_text: &'a str,
    pub files: &'a [String],
    pub artifacts: &'a [ArtifactMetadata],
    pub completed_at: DateTime<Utc>,
    pub manifest_seed: Option<&'a ConversationRecoveryManifest>,
}

pub(super) struct TerminalFailure<'a> {
    pub status: JobStatus,
    pub persisted_message: &'a str,
    pub event_message: &'a str,
    pub conversation_id: Option<&'a str>,
    pub current_url: Option<&'a str>,
    pub output_text: Option<&'a str>,
    pub record_recovery_metadata: bool,
    pub recovery_trace: Option<&'a RecoveryTrace>,
    pub recovery_packet: Option<&'a RecoveryPacket>,
}

impl JobLifecycle {
    pub fn ask() -> Self {
        Self { kind: JobKind::Ask }
    }

    pub fn research() -> Self {
        Self {
            kind: JobKind::Research,
        }
    }

    pub fn queued_message(self) -> &'static str {
        match self.kind {
            JobKind::Ask => "queued ask job",
            JobKind::Research => "queued research job",
        }
    }

    pub fn cancelled_before_execution_message(self) -> &'static str {
        match self.kind {
            JobKind::Ask => "ask cancelled before execution",
            JobKind::Research => "research cancelled before execution",
        }
    }

    pub fn completed_message(self, recovered: bool) -> &'static str {
        match (self.kind, recovered) {
            (JobKind::Ask, true) => "ask recovered",
            (JobKind::Ask, false) => "ask completed",
            (JobKind::Research, true) => "research recovered",
            (JobKind::Research, false) => "research completed",
        }
    }
}

impl<'a> JobLifecycleRef<'a> {
    pub fn queued_event(self) -> EventPayload {
        self.state_event(
            JobStatus::Queued,
            Some(self.lifecycle.queued_message().to_owned()),
            self.conversation_id.map(ToOwned::to_owned),
            self.created_at,
            vec![],
        )
    }

    pub fn state_event(
        self,
        status: JobStatus,
        message: Option<String>,
        conversation_id: Option<String>,
        updated_at: DateTime<Utc>,
        artifacts: Vec<ArtifactMetadata>,
    ) -> EventPayload {
        EventPayload::JobState(JobState {
            job_id: self.job_id.to_owned(),
            status,
            message,
            conversation_id,
            updated_at,
            artifacts,
        })
    }
}

pub(super) fn ensure_prompt_execution_allowed(config: &AppConfig) -> Result<()> {
    if config.safe_mode() {
        anyhow::bail!(
            "safe mode enabled: prompt execution is disabled; run doctor or capabilities only"
        );
    }
    Ok(())
}

pub(super) fn prepare_job_record(
    db: &Database,
    input: PrepareJobInput<'_>,
) -> Result<PreparedJobRecord> {
    let created_at = Utc::now();
    let job_id = format!("job-{}", created_at.timestamp_millis());
    let conversation_id =
        resolve_conversation_id(db, input.new_conversation, input.conversation_id)?;
    let lifecycle = JobLifecycle { kind: input.kind };

    db.insert_job(
        &job_id,
        JobStatus::Queued,
        Some(lifecycle.queued_message()),
        conversation_id.as_deref(),
        created_at,
        created_at,
    )?;
    db.upsert_job_detail(&job_id, input.kind, Some(input.input_text), None)?;

    Ok(PreparedJobRecord {
        job_id,
        conversation_id,
        created_at,
    })
}

pub(super) fn emit_job_state<F>(
    db: &Database,
    job: JobLifecycleRef<'_>,
    status: JobStatus,
    message: &'static str,
    conversation_id: Option<&str>,
    on_event: &mut F,
) -> Result<()>
where
    F: FnMut(EventPayload) -> Result<()>,
{
    db.insert_job(
        job.job_id,
        status,
        Some(message),
        conversation_id.or(job.conversation_id),
        job.created_at,
        Utc::now(),
    )?;
    on_event(
        job.state_event(
            status,
            Some(message.to_owned()),
            conversation_id
                .or(job.conversation_id)
                .map(ToOwned::to_owned),
            Utc::now(),
            vec![],
        ),
    )
}

pub(super) fn final_status_and_message(
    lifecycle: JobLifecycle,
    recovered: bool,
) -> (JobStatus, &'static str) {
    (
        if recovered {
            JobStatus::Recovered
        } else {
            JobStatus::Completed
        },
        lifecycle.completed_message(recovered),
    )
}

pub(super) fn status_for_error_message(message: &str) -> JobStatus {
    if is_cancelled_message(message) {
        JobStatus::Cancelled
    } else {
        JobStatus::Failed
    }
}

pub(super) fn persist_artifacts<F>(
    db: &Database,
    job_id: &str,
    artifacts: &[ArtifactMetadata],
    completed_at: DateTime<Utc>,
    on_event: &mut F,
) -> Result<()>
where
    F: FnMut(EventPayload) -> Result<()>,
{
    for artifact in artifacts {
        db.insert_artifact(artifact, completed_at)?;
        db.link_job_artifact(job_id, &artifact.artifact_id)?;
        on_event(EventPayload::ArtifactReady(artifact.clone()))?;
    }
    db.prune_artifacts(ARTIFACT_RETENTION_LIMIT)?;
    Ok(())
}

pub(super) fn record_failure_checkpoint(
    db: &Database,
    job_id: &str,
    failed_at: DateTime<Utc>,
    message: &str,
    current_url: Option<&str>,
    recovery_trace: Option<&RecoveryTrace>,
    recovery_packet: Option<&RecoveryPacket>,
) -> Result<()> {
    db.record_job_failure(job_id, failed_at, message, current_url)?;
    db.update_job_output(job_id, None, current_url, None)?;
    db.update_job_recovery_metadata(job_id, recovery_trace, recovery_packet)
}

pub(super) fn record_terminal_failure<F>(
    db: &Database,
    job: JobLifecycleRef<'_>,
    failure: TerminalFailure<'_>,
    on_event: &mut F,
) -> Result<()>
where
    F: FnMut(EventPayload) -> Result<()>,
{
    let failed_at = Utc::now();
    db.insert_job(
        job.job_id,
        failure.status,
        Some(failure.persisted_message),
        failure.conversation_id.or(job.conversation_id),
        job.created_at,
        failed_at,
    )?;
    db.record_job_failure(
        job.job_id,
        failed_at,
        failure.persisted_message,
        failure.current_url,
    )?;
    db.update_job_output(job.job_id, failure.output_text, failure.current_url, None)?;
    if failure.record_recovery_metadata {
        db.update_job_recovery_metadata(
            job.job_id,
            failure.recovery_trace,
            failure.recovery_packet,
        )?;
    }
    on_event(
        job.state_event(
            failure.status,
            Some(failure.event_message.to_owned()),
            failure
                .conversation_id
                .or(job.conversation_id)
                .map(ToOwned::to_owned),
            failed_at,
            vec![],
        ),
    )
}

fn resolve_conversation_id(
    db: &Database,
    new_conversation: bool,
    conversation_id: Option<String>,
) -> Result<Option<String>> {
    match (new_conversation, conversation_id) {
        (true, _) => Ok(None),
        (false, Some(value)) => {
            if db.find_conversation(&value)?.is_none() {
                anyhow::bail!("conversation {value} was not found");
            }
            Ok(Some(value))
        }
        (false, None) => Ok(None),
    }
}

impl CoreService {
    pub(super) fn cancel_if_requested_before_execution<F>(
        &self,
        db: &Database,
        job: JobLifecycleRef<'_>,
        on_event: &mut F,
    ) -> Result<()>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        if !db.job_cancel_requested(job.job_id)? {
            return Ok(());
        }

        let message = job.lifecycle.cancelled_before_execution_message();
        self.cancel_job_with_detail(job.job_id, job.conversation_id, message)?;
        on_event(job.state_event(
            JobStatus::Cancelled,
            Some(message.to_owned()),
            job.conversation_id.map(ToOwned::to_owned),
            Utc::now(),
            vec![],
        ))?;
        anyhow::bail!(message);
    }

    pub(super) fn cleanup_cancel_registration_best_effort(&self, db: &Database, job_id: &str) {
        let _ = lock_cancel_registry(&self.cancel_registry).map(|mut set| {
            set.remove(job_id);
        });
        let _ = db.set_job_cancel_requested(job_id, false);
    }

    pub(super) fn clear_cancel_requested(&self, db: &Database, job_id: &str) -> Result<()> {
        db.set_job_cancel_requested(job_id, false)?;
        let _ = lock_cancel_registry(&self.cancel_registry).map(|mut set| {
            set.remove(job_id);
        });
        Ok(())
    }

    pub(super) fn finalize_session_and_conversation(
        &self,
        db: &Database,
        input: ConversationFinalization<'_>,
    ) -> Result<()> {
        let browser_snapshot = lock_browser(&self.browser)?.health()?;
        let (session_id, session_label) = browser_session_identity(&browser_snapshot);
        db.upsert_session(
            &session_id,
            &session_label,
            Some(input.current_url),
            input.completed_at,
        )?;
        let existing_manifest = db.conversation_recovery_manifest(input.conversation_id)?;
        let conversation_manifest = update_conversation_recovery_manifest(
            existing_manifest.as_ref().or(input.manifest_seed),
            input.conversation_id,
            input.prompt,
            input.output_text,
            input.files,
            input.artifacts,
            input.completed_at,
        );
        db.upsert_conversation(
            input.conversation_id,
            &session_id,
            "Browser conversation",
            Some(input.current_url),
            Some(input.prompt),
            Some(input.output_text),
            Some(&conversation_manifest),
            input.completed_at,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppConfig, Database};
    use tempfile::tempdir;

    #[test]
    fn queued_event_matches_persisted_ask_job() {
        let dir = tempdir().unwrap();
        let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        let db = Database::open(&config.paths.db_path).unwrap();
        let prepared = prepare_job_record(
            &db,
            PrepareJobInput {
                kind: JobKind::Ask,
                input_text: "hello",
                conversation_id: None,
                new_conversation: false,
            },
        )
        .unwrap();
        let job = JobLifecycleRef {
            job_id: &prepared.job_id,
            created_at: prepared.created_at,
            conversation_id: prepared.conversation_id.as_deref(),
            lifecycle: JobLifecycle::ask(),
        };

        let EventPayload::JobState(event) = job.queued_event() else {
            panic!("expected job state");
        };
        let detail = db.job_detail(&prepared.job_id).unwrap().unwrap();

        assert_eq!(event.job_id, prepared.job_id);
        assert_eq!(event.status, JobStatus::Queued);
        assert_eq!(event.message.as_deref(), Some("queued ask job"));
        assert_eq!(event.updated_at, prepared.created_at);
        assert_eq!(detail.status, event.status);
        assert_eq!(detail.message, event.message);
        assert_eq!(detail.created_at, prepared.created_at);
    }

    #[test]
    fn queued_event_matches_persisted_research_job() {
        let dir = tempdir().unwrap();
        let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        let db = Database::open(&config.paths.db_path).unwrap();
        let prepared = prepare_job_record(
            &db,
            PrepareJobInput {
                kind: JobKind::Research,
                input_text: "investigate",
                conversation_id: None,
                new_conversation: false,
            },
        )
        .unwrap();
        let job = JobLifecycleRef {
            job_id: &prepared.job_id,
            created_at: prepared.created_at,
            conversation_id: prepared.conversation_id.as_deref(),
            lifecycle: JobLifecycle::research(),
        };

        let EventPayload::JobState(event) = job.queued_event() else {
            panic!("expected job state");
        };
        let detail = db.job_detail(&prepared.job_id).unwrap().unwrap();

        assert_eq!(event.status, JobStatus::Queued);
        assert_eq!(event.message.as_deref(), Some("queued research job"));
        assert_eq!(detail.kind, JobKind::Research);
        assert_eq!(detail.status, event.status);
        assert_eq!(detail.message, event.message);
    }

    #[test]
    fn safe_mode_blocks_prompt_execution_before_validation() {
        let dir = tempdir().unwrap();
        let mut config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        config.file.safe_mode = Some(true);

        let err = ensure_prompt_execution_allowed(&config).unwrap_err();

        assert_eq!(
            format!("{err:#}"),
            "safe mode enabled: prompt execution is disabled; run doctor or capabilities only"
        );
    }

    #[test]
    fn terminal_status_maps_cancelled_messages() {
        assert_eq!(
            status_for_error_message("ask cancelled: client disconnected"),
            JobStatus::Cancelled
        );
        assert_eq!(
            status_for_error_message("research job failed"),
            JobStatus::Failed
        );
    }

    #[test]
    fn completed_message_preserves_job_kind_labels() {
        assert_eq!(
            JobLifecycle::ask().completed_message(false),
            "ask completed"
        );
        assert_eq!(JobLifecycle::ask().completed_message(true), "ask recovered");
        assert_eq!(
            JobLifecycle::research().completed_message(false),
            "research completed"
        );
        assert_eq!(
            JobLifecycle::research().completed_message(true),
            "research recovered"
        );
    }

    #[test]
    fn prepare_job_rejects_missing_conversation() {
        let dir = tempdir().unwrap();
        let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        let db = Database::open(&config.paths.db_path).unwrap();

        let err = prepare_job_record(
            &db,
            PrepareJobInput {
                kind: JobKind::Ask,
                input_text: "hello",
                conversation_id: Some("missing".to_owned()),
                new_conversation: false,
            },
        )
        .unwrap_err();

        assert_eq!(format!("{err:#}"), "conversation missing was not found");
    }
}
