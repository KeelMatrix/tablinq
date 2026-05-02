use super::lifecycle::{
    ConversationFinalization, JobLifecycle, JobLifecycleRef, PrepareJobInput, TerminalFailure,
    emit_job_state, ensure_prompt_execution_allowed, final_status_and_message, persist_artifacts,
    prepare_job_record, record_terminal_failure, status_for_error_message,
};
use super::provider_failure_recovery::{
    ProviderFailureRecoveryInput, ProviderFailureRecoveryOutcome,
};
use super::support::{conversation_id_from_url, lock_browser, lock_cancel_registry};
use super::{CoreService, ExecutionRecoveryState};
use crate::browser::execution_error_metadata;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use shared_protocol::{AskCommand, AskResult, EventPayload, JobKind, JobState, JobStatus};
use std::cell::RefCell;

#[derive(Debug, Clone)]
pub struct PreparedAskJob {
    pub job_id: String,
    pub command: AskCommand,
    pub created_at: DateTime<Utc>,
}

impl PreparedAskJob {
    fn lifecycle_ref(&self) -> JobLifecycleRef<'_> {
        JobLifecycleRef {
            job_id: &self.job_id,
            created_at: self.created_at,
            conversation_id: self.command.conversation_id.as_deref(),
            lifecycle: JobLifecycle::ask(),
        }
    }

    pub(crate) fn queued_event(&self) -> EventPayload {
        self.lifecycle_ref().queued_event()
    }
}

impl CoreService {
    pub fn prepare_ask(&self, command: AskCommand) -> Result<PreparedAskJob> {
        ensure_prompt_execution_allowed(&self.config)?;
        command.validate().context("validate ask command")?;

        let db = self.open_db()?;
        let prepared = prepare_job_record(
            &db,
            PrepareJobInput {
                kind: JobKind::Ask,
                input_text: &command.prompt,
                conversation_id: command.conversation_id.clone(),
                new_conversation: command.new_conversation,
            },
        )?;
        let mut effective = command.clone();
        effective.conversation_id = prepared.conversation_id.clone();
        effective.new_conversation = command.new_conversation;

        Ok(PreparedAskJob {
            job_id: prepared.job_id,
            command: effective,
            created_at: prepared.created_at,
        })
    }

    pub fn execute_prepared_ask<F>(
        &self,
        prepared: &PreparedAskJob,
        mut on_event: F,
    ) -> Result<AskResult>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let db = self.open_db()?;
        self.cancel_if_requested_before_execution(&db, prepared.lifecycle_ref(), &mut on_event)?;
        emit_job_state(
            &db,
            prepared.lifecycle_ref(),
            JobStatus::Starting,
            "starting ask",
            prepared.command.conversation_id.as_deref(),
            &mut on_event,
        )?;
        emit_job_state(
            &db,
            prepared.lifecycle_ref(),
            JobStatus::Running,
            "streaming response",
            prepared.command.conversation_id.as_deref(),
            &mut on_event,
        )?;

        let recovery_state = RefCell::new(ExecutionRecoveryState::default());
        let latest_lifecycle_update = RefCell::new(None::<crate::browser::RequestLifecycleUpdate>);
        let event_sink = RefCell::new(&mut on_event);
        let mut recovery_trace: Option<shared_protocol::RecoveryTrace> = None;
        let mut recovery_packet: Option<shared_protocol::RecoveryPacket> = None;
        let mut recovery_attempt_count = 0u32;
        let mut current_command = prepared.command.clone();
        let outcome = loop {
            latest_lifecycle_update.replace(None);
            let outcome = lock_browser(&self.browser)?.ask(
                &prepared.job_id,
                &current_command,
                |event| {
                    let mut sink = event_sink.borrow_mut();
                    (*sink)(event)
                },
                |update| {
                    latest_lifecycle_update.replace(Some(update.clone()));
                    self.apply_request_lifecycle_update(
                        &db,
                        &prepared.job_id,
                        prepared.created_at,
                        prepared.command.conversation_id.as_deref(),
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

            match outcome {
                Ok(mut result) => {
                    if let Some(trace) = recovery_trace.as_mut() {
                        trace.replacement_conversation_id = Some(result.conversation_id.clone());
                    }
                    result.recovery_trace = recovery_trace.clone();
                    result.recovery_packet = recovery_packet.clone();
                    break Ok(result);
                }
                Err(err) => {
                    let latest_update = latest_lifecycle_update.borrow().clone();
                    let recovery_outcome = {
                        let mut sink = event_sink.borrow_mut();
                        self.handle_provider_failure_recovery(
                            ProviderFailureRecoveryInput {
                                db: &db,
                                prepared,
                                current_command: &current_command,
                                latest_update,
                                err,
                                recovery_trace: recovery_trace.as_ref(),
                                recovery_packet: recovery_packet.as_ref(),
                                recovery_attempt_count,
                            },
                            &mut **sink,
                        )?
                    };

                    match recovery_outcome {
                        ProviderFailureRecoveryOutcome::Replay {
                            trace,
                            packet,
                            next_command,
                            attempt_count,
                        } => {
                            recovery_trace = Some(trace);
                            recovery_packet = Some(packet);
                            recovery_attempt_count = attempt_count;
                            current_command = next_command;
                        }
                        ProviderFailureRecoveryOutcome::TerminalError(err) => break Err(err),
                    }
                }
            }
        };

        self.cleanup_cancel_registration_best_effort(&db, &prepared.job_id);

        match outcome {
            Ok(mut result) => {
                let recovery_state = recovery_state.into_inner();
                let provider_recovered = result
                    .recovery_trace
                    .as_ref()
                    .map(|trace| trace.recovery_attempted)
                    .unwrap_or(false);
                result.low_confidence = recovery_state.recovered || provider_recovered;
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
                    recovery_state.recovered || provider_recovered,
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
                        "reconciled the interrupted ask against the live conversation",
                        Some(&result.current_url),
                    )?;
                }
                if provider_recovered {
                    db.record_job_recovery(
                        &prepared.job_id,
                        completed_at,
                        "reloaded the provider tab and replayed the ask in a fresh conversation",
                        Some(&result.current_url),
                    )?;
                }
                self.finalize_session_and_conversation(
                    &db,
                    ConversationFinalization {
                        conversation_id: &result.conversation_id,
                        current_url: &result.current_url,
                        prompt: &prepared.command.prompt,
                        output_text: &result.output_text,
                        files: &prepared.command.files,
                        artifacts: &result.artifacts,
                        completed_at,
                        manifest_seed: result
                            .recovery_packet
                            .as_ref()
                            .and_then(|packet| packet.context_manifest.as_ref()),
                    },
                )?;
                db.update_job_output(
                    &prepared.job_id,
                    Some(&result.output_text),
                    Some(&result.current_url),
                    None,
                )?;
                db.update_job_recovery_metadata(
                    &prepared.job_id,
                    result.recovery_trace.as_ref(),
                    result.recovery_packet.as_ref(),
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
                let typed_error = execution_error_metadata(&err);
                let message = format!("{err:#}");
                let status = status_for_error_message(&message);
                let current_url = typed_error
                    .as_ref()
                    .and_then(|error| error.current_url.as_deref());
                let failure_conversation_id = typed_error
                    .as_ref()
                    .and_then(|error| {
                        error
                            .recovery_trace
                            .as_ref()
                            .and_then(|trace| {
                                trace
                                    .replacement_conversation_id
                                    .clone()
                                    .or_else(|| trace.prior_conversation_id.clone())
                            })
                            .or_else(|| {
                                error
                                    .current_url
                                    .as_deref()
                                    .and_then(conversation_id_from_url)
                            })
                    })
                    .or_else(|| prepared.command.conversation_id.clone());
                record_terminal_failure(
                    &db,
                    prepared.lifecycle_ref(),
                    TerminalFailure {
                        status,
                        persisted_message: &message,
                        event_message: &message,
                        conversation_id: failure_conversation_id.as_deref(),
                        current_url,
                        output_text: None,
                        record_recovery_metadata: typed_error.is_some(),
                        recovery_trace: typed_error
                            .as_ref()
                            .and_then(|error| error.recovery_trace.as_ref()),
                        recovery_packet: typed_error
                            .as_ref()
                            .and_then(|error| error.recovery_packet.as_ref()),
                    },
                    &mut on_event,
                )?;
                Err(err)
            }
        }
    }
}
