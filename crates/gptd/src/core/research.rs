use super::recovery_manifest::update_conversation_recovery_manifest;
use super::support::{
    apply_research_update, browser_session_identity, is_cancelled_message, lock_browser,
    lock_cancel_registry, maybe_write_research_output,
};
use super::{ARTIFACT_RETENTION_LIMIT, CoreService, ExecutionRecoveryState};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use shared_protocol::{
    EventPayload, JobKind, JobState, JobStatus, ResearchCommand, ResearchResult,
};
use std::cell::{Cell, RefCell};

#[derive(Debug, Clone)]
pub struct PreparedResearchJob {
    pub job_id: String,
    pub command: ResearchCommand,
    pub created_at: DateTime<Utc>,
}

impl CoreService {
    pub fn prepare_research(&self, command: ResearchCommand) -> Result<PreparedResearchJob> {
        if self.config.safe_mode() {
            anyhow::bail!(
                "safe mode enabled: prompt execution is disabled; run doctor or capabilities only"
            );
        }
        command.validate().context("validate research command")?;

        let db = self.open_db()?;
        let created_at = Utc::now();
        let job_id = format!("job-{}", created_at.timestamp_millis());
        let conversation_id = match (command.new_conversation, command.conversation_id.clone()) {
            (true, _) => None,
            (false, Some(value)) => {
                if db.find_conversation(&value)?.is_none() {
                    anyhow::bail!("conversation {value} was not found");
                }
                Some(value)
            }
            (false, None) => None,
        };
        let mut effective = command.clone();
        effective.conversation_id = conversation_id.clone();
        effective.new_conversation = false;
        db.insert_job(
            &job_id,
            JobStatus::Queued,
            Some("queued research job"),
            conversation_id.as_deref(),
            created_at,
            created_at,
        )?;
        db.upsert_job_detail(&job_id, JobKind::Research, Some(&effective.query), None)?;

        Ok(PreparedResearchJob {
            job_id,
            command: effective,
            created_at,
        })
    }

    pub fn execute_prepared_research<F>(
        &self,
        prepared: &PreparedResearchJob,
        mut on_event: F,
    ) -> Result<ResearchResult>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let db = self.open_db()?;
        if db.job_cancel_requested(&prepared.job_id)? {
            let message = "research cancelled before execution";
            self.cancel_job_with_detail(
                &prepared.job_id,
                prepared.command.conversation_id.as_deref(),
                message,
            )?;
            on_event(EventPayload::JobState(JobState {
                job_id: prepared.job_id.clone(),
                status: JobStatus::Cancelled,
                message: Some(message.to_owned()),
                conversation_id: prepared.command.conversation_id.clone(),
                updated_at: Utc::now(),
                artifacts: vec![],
            }))?;
            anyhow::bail!(message);
        }

        let conversation_id = RefCell::new(None);
        let current_url = RefCell::new(None);
        let report_text = RefCell::new(String::new());
        let next_seq = Cell::new(0u64);
        let recovery_state = RefCell::new(ExecutionRecoveryState::default());
        let event_sink = RefCell::new(&mut on_event);

        let outcome = lock_browser(&self.browser)?.research(
            &prepared.job_id,
            &prepared.command,
            |update| {
                let mut conversation_id_ref = conversation_id.borrow_mut();
                let mut current_url_ref = current_url.borrow_mut();
                let mut report_text_ref = report_text.borrow_mut();
                let mut next_seq_value = next_seq.get();
                apply_research_update(
                    &db,
                    &prepared.job_id,
                    prepared.created_at,
                    &mut conversation_id_ref,
                    &mut current_url_ref,
                    &mut report_text_ref,
                    &mut next_seq_value,
                    update,
                    &mut |event| {
                        let mut sink = event_sink.borrow_mut();
                        (*sink)(event)
                    },
                )?;
                next_seq.set(next_seq_value);
                Ok(())
            },
            |update| {
                self.apply_request_lifecycle_update(
                    &db,
                    &prepared.job_id,
                    prepared.created_at,
                    conversation_id
                        .borrow()
                        .as_deref()
                        .or(prepared.command.conversation_id.as_deref()),
                    update,
                    &recovery_state,
                    &event_sink,
                )
            },
            || {
                lock_cancel_registry(&self.cancel_registry)
                    .map(|set| set.contains(&prepared.job_id))
                    .unwrap_or(false)
            },
        );

        let _ = lock_cancel_registry(&self.cancel_registry).map(|mut set| {
            set.remove(&prepared.job_id);
        });
        let _ = db.set_job_cancel_requested(&prepared.job_id, false);

        match outcome {
            Ok(mut result) => {
                let recovery_state = recovery_state.into_inner();
                result.low_confidence = recovery_state.recovered;
                let output_path = maybe_write_research_output(
                    &prepared.command,
                    &prepared.job_id,
                    &result.report_text,
                )?;
                result.output_path = output_path.clone();
                let completed_at = result.completed_at;
                for artifact in &result.artifacts {
                    db.insert_artifact(artifact, completed_at)?;
                    db.link_job_artifact(&prepared.job_id, &artifact.artifact_id)?;
                    on_event(EventPayload::ArtifactReady(artifact.clone()))?;
                }
                db.prune_artifacts(ARTIFACT_RETENTION_LIMIT)?;
                let final_status = if recovery_state.recovered {
                    JobStatus::Recovered
                } else {
                    JobStatus::Completed
                };
                let final_message = if recovery_state.recovered {
                    "research recovered"
                } else {
                    "research completed"
                };
                db.insert_job(
                    &prepared.job_id,
                    final_status,
                    Some(final_message),
                    Some(&result.conversation_id),
                    prepared.created_at,
                    completed_at,
                )?;
                if recovery_state.recovered {
                    db.record_job_recovery(
                        &prepared.job_id,
                        completed_at,
                        "reconciled the interrupted research job against the live conversation",
                        Some(&result.current_url),
                    )?;
                }
                db.update_job_output(
                    &prepared.job_id,
                    Some(&result.report_text),
                    Some(&result.current_url),
                    output_path.as_deref(),
                )?;
                let browser_snapshot = lock_browser(&self.browser)?.health()?;
                let (session_id, session_label) = browser_session_identity(&browser_snapshot);
                db.upsert_session(
                    &session_id,
                    &session_label,
                    Some(&result.current_url),
                    completed_at,
                )?;
                let existing_manifest =
                    db.conversation_recovery_manifest(&result.conversation_id)?;
                let conversation_manifest = update_conversation_recovery_manifest(
                    existing_manifest.as_ref(),
                    &result.conversation_id,
                    &prepared.command.query,
                    &result.report_text,
                    &prepared.command.files,
                    &result.artifacts,
                    completed_at,
                );
                db.upsert_conversation(
                    &result.conversation_id,
                    &session_id,
                    "Browser conversation",
                    Some(&result.current_url),
                    Some(&prepared.command.query),
                    Some(&result.report_text),
                    Some(&conversation_manifest),
                    completed_at,
                )?;
                on_event(EventPayload::JobState(JobState {
                    job_id: prepared.job_id.clone(),
                    status: final_status,
                    message: Some(final_message.to_owned()),
                    conversation_id: Some(result.conversation_id.clone()),
                    updated_at: completed_at,
                    artifacts: result.artifacts.clone(),
                }))?;
                Ok(result)
            }
            Err(err) => {
                let conversation_id = conversation_id.into_inner();
                let current_url = current_url.into_inner();
                let report_text = report_text.into_inner();
                let failed_at = Utc::now();
                let message = format!("{err:#}");
                let status = if is_cancelled_message(&message) {
                    JobStatus::Cancelled
                } else {
                    JobStatus::Failed
                };
                let status_message = if status == JobStatus::Cancelled {
                    "research cancelled"
                } else {
                    "research failed"
                };
                db.insert_job(
                    &prepared.job_id,
                    status,
                    Some(&message),
                    conversation_id.as_deref(),
                    prepared.created_at,
                    failed_at,
                )?;
                db.record_job_failure(
                    &prepared.job_id,
                    failed_at,
                    &message,
                    current_url.as_deref(),
                )?;
                db.update_job_output(
                    &prepared.job_id,
                    if report_text.is_empty() {
                        None
                    } else {
                        Some(report_text.as_str())
                    },
                    current_url.as_deref(),
                    None,
                )?;
                on_event(EventPayload::JobState(JobState {
                    job_id: prepared.job_id.clone(),
                    status,
                    message: Some(status_message.to_owned()),
                    conversation_id,
                    updated_at: failed_at,
                    artifacts: vec![],
                }))?;
                Err(err)
            }
        }
    }
}
