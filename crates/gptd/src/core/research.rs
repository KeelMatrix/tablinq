use super::lifecycle::{
    ConversationFinalization, JobLifecycle, JobLifecycleRef, PrepareJobInput, TerminalFailure,
    ensure_prompt_execution_allowed, final_status_and_message, persist_artifacts,
    prepare_job_record, record_terminal_failure, status_for_error_message,
};
use super::support::{
    apply_research_update, lock_browser, lock_cancel_registry, maybe_write_research_output,
};
use super::{CoreService, ExecutionRecoveryState};
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

impl PreparedResearchJob {
    fn lifecycle_ref(&self) -> JobLifecycleRef<'_> {
        JobLifecycleRef {
            job_id: &self.job_id,
            created_at: self.created_at,
            conversation_id: self.command.conversation_id.as_deref(),
            lifecycle: JobLifecycle::research(),
        }
    }

    pub(crate) fn queued_event(&self) -> EventPayload {
        self.lifecycle_ref().queued_event()
    }
}

impl CoreService {
    pub fn prepare_research(&self, command: ResearchCommand) -> Result<PreparedResearchJob> {
        ensure_prompt_execution_allowed(&self.config)?;
        command.validate().context("validate research command")?;

        let db = self.open_db()?;
        let prepared = prepare_job_record(
            &db,
            PrepareJobInput {
                kind: JobKind::Research,
                input_text: &command.query,
                conversation_id: command.conversation_id.clone(),
                new_conversation: command.new_conversation,
            },
        )?;
        let mut effective = command.clone();
        effective.conversation_id = prepared.conversation_id.clone();
        effective.new_conversation = false;

        Ok(PreparedResearchJob {
            job_id: prepared.job_id,
            command: effective,
            created_at: prepared.created_at,
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
        self.cancel_if_requested_before_execution(&db, prepared.lifecycle_ref(), &mut on_event)?;

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

        self.cleanup_cancel_registration_best_effort(&db, &prepared.job_id);

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
                persist_artifacts(
                    &db,
                    &prepared.job_id,
                    &result.artifacts,
                    completed_at,
                    &mut on_event,
                )?;
                let (final_status, final_message) = final_status_and_message(
                    prepared.lifecycle_ref().lifecycle,
                    recovery_state.recovered,
                );
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
                self.finalize_session_and_conversation(
                    &db,
                    ConversationFinalization {
                        conversation_id: &result.conversation_id,
                        current_url: &result.current_url,
                        prompt: &prepared.command.query,
                        output_text: &result.report_text,
                        files: &prepared.command.files,
                        artifacts: &result.artifacts,
                        completed_at,
                        manifest_seed: None,
                    },
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
                let message = format!("{err:#}");
                let status = status_for_error_message(&message);
                let status_message = if status == JobStatus::Cancelled {
                    "research cancelled"
                } else {
                    "research failed"
                };
                record_terminal_failure(
                    &db,
                    prepared.lifecycle_ref(),
                    TerminalFailure {
                        status,
                        persisted_message: &message,
                        event_message: status_message,
                        conversation_id: conversation_id.as_deref(),
                        current_url: current_url.as_deref(),
                        output_text: if report_text.is_empty() {
                            None
                        } else {
                            Some(report_text.as_str())
                        },
                        record_recovery_metadata: false,
                        recovery_trace: None,
                        recovery_packet: None,
                    },
                    &mut on_event,
                )?;
                Err(err)
            }
        }
    }
}
