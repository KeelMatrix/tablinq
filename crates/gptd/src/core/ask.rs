use super::recovery::{
    ProviderGenerationRecoveryPacketInput, build_provider_generation_context_summary,
    build_provider_generation_recovery_packet, build_provider_generation_recovery_trace,
    build_provider_generation_replay_files, build_provider_generation_replay_prompt,
    provider_recovery_attempt_request_id, should_auto_recover_provider_generation_failure,
};
use super::recovery_manifest::{
    update_conversation_recovery_manifest, write_provider_generation_recovery_manifest_attachment,
};
use super::support::{
    browser_session_identity, conversation_id_from_url, is_cancelled_message, lock_browser,
    lock_cancel_registry,
};
use super::{
    ARTIFACT_RETENTION_LIMIT, CoreService, ExecutionRecoveryState, error_code_for_message,
};
use crate::browser::{execution_error, execution_error_metadata};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use shared_protocol::{
    AskCommand, AskResult, ErrorCode, EventPayload, JobKind, JobState, JobStatus,
};
use std::cell::RefCell;

#[derive(Debug, Clone)]
pub struct PreparedAskJob {
    pub job_id: String,
    pub command: AskCommand,
    pub created_at: DateTime<Utc>,
}

impl CoreService {
    pub fn prepare_ask(&self, command: AskCommand) -> Result<PreparedAskJob> {
        if self.config.safe_mode() {
            anyhow::bail!(
                "safe mode enabled: prompt execution is disabled; run doctor or capabilities only"
            );
        }
        command.validate().context("validate ask command")?;

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
        effective.new_conversation = command.new_conversation;

        db.insert_job(
            &job_id,
            JobStatus::Queued,
            Some("queued ask job"),
            effective.conversation_id.as_deref(),
            created_at,
            created_at,
        )?;
        db.upsert_job_detail(&job_id, JobKind::Ask, Some(&effective.prompt), None)?;

        Ok(PreparedAskJob {
            job_id,
            command: effective,
            created_at,
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
        if db.job_cancel_requested(&prepared.job_id)? {
            let message = "ask cancelled before execution";
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

        db.insert_job(
            &prepared.job_id,
            JobStatus::Starting,
            Some("starting ask"),
            prepared.command.conversation_id.as_deref(),
            prepared.created_at,
            Utc::now(),
        )?;
        on_event(EventPayload::JobState(JobState {
            job_id: prepared.job_id.clone(),
            status: JobStatus::Starting,
            message: Some("starting ask".to_owned()),
            conversation_id: prepared.command.conversation_id.clone(),
            updated_at: Utc::now(),
            artifacts: vec![],
        }))?;

        db.insert_job(
            &prepared.job_id,
            JobStatus::Running,
            Some("streaming response"),
            prepared.command.conversation_id.as_deref(),
            prepared.created_at,
            Utc::now(),
        )?;
        on_event(EventPayload::JobState(JobState {
            job_id: prepared.job_id.clone(),
            status: JobStatus::Running,
            message: Some("streaming response".to_owned()),
            conversation_id: prepared.command.conversation_id.clone(),
            updated_at: Utc::now(),
            artifacts: vec![],
        }))?;

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
                    let current_url = latest_update
                        .as_ref()
                        .and_then(|update| update.current_url.clone());
                    let typed_error = execution_error_metadata(&err);
                    if !matches!(
                        typed_error.as_ref().map(|error| error.error_code),
                        Some(ErrorCode::ProviderGenerationFailedNonretryable)
                    ) {
                        if let (Some(trace), Some(packet)) =
                            (recovery_trace.clone(), recovery_packet.clone())
                        {
                            let message = format!("{err:#}");
                            break Err(execution_error(
                                crate::browser::ExecutionErrorMetadata::new(
                                    error_code_for_message(&message),
                                    message,
                                    current_url,
                                )
                                .with_recovery(trace, packet),
                            ));
                        }
                        break Err(err);
                    }

                    let provider_error = typed_error.expect("provider failure metadata missing");
                    let prior_conversation_id = latest_update
                        .as_ref()
                        .and_then(|update| update.conversation_id.clone())
                        .or_else(|| current_command.conversation_id.clone())
                        .or_else(|| {
                            provider_error
                                .current_url
                                .as_deref()
                                .and_then(conversation_id_from_url)
                        });
                    let prior_conversation =
                        if let Some(conversation_id) = prior_conversation_id.as_deref() {
                            db.find_conversation(conversation_id)?
                        } else {
                            None
                        };
                    let prior_manifest =
                        if let Some(conversation_id) = prior_conversation_id.as_deref() {
                            db.conversation_recovery_manifest(conversation_id)?
                        } else {
                            None
                        };
                    let recovery_allowed = should_auto_recover_provider_generation_failure(
                        prepared.command.auto_recover_provider_generation_failure,
                        recovery_attempt_count,
                    );
                    let recovery_attempted = recovery_attempt_count > 0 || recovery_allowed;
                    let effective_attempt_count = if recovery_allowed {
                        recovery_attempt_count + 1
                    } else {
                        recovery_attempt_count
                    };
                    let prior_request_id =
                        Some(provider_recovery_attempt_request_id(&prepared.job_id, 0));
                    let failure_bundle_id = if let Some(trace) = recovery_trace.as_ref() {
                        trace.failure_bundle_id.clone()
                    } else {
                        self.capture_failure_bundle(
                            Some(prepared.command.mode),
                            &provider_error.message,
                            None,
                        )
                        .ok()
                        .map(|bundle| bundle.bundle_id)
                    };
                    let context_summary = build_provider_generation_context_summary(
                        prior_manifest.as_ref(),
                        prior_conversation.as_ref(),
                        prior_conversation_id.as_deref(),
                    );
                    let previous_recovery_packet = recovery_packet.as_ref().cloned();
                    let recovery_manifest_path = if recovery_allowed {
                        prior_manifest
                            .as_ref()
                            .map(|manifest| {
                                write_provider_generation_recovery_manifest_attachment(
                                    &self.config.paths.state_root,
                                    &prepared.job_id,
                                    effective_attempt_count,
                                    manifest,
                                )
                            })
                            .transpose()?
                    } else {
                        previous_recovery_packet
                            .as_ref()
                            .and_then(|packet| packet.recovery_manifest_path.clone())
                    };
                    let replay_files = if recovery_allowed {
                        build_provider_generation_replay_files(
                            &prepared.command.files,
                            prior_manifest.as_ref(),
                            recovery_manifest_path.as_deref(),
                        )
                    } else {
                        previous_recovery_packet
                            .as_ref()
                            .map(|packet| packet.replay_files.clone())
                            .unwrap_or_else(|| prepared.command.files.clone())
                    };
                    let mut trace = build_provider_generation_recovery_trace(
                        failure_bundle_id,
                        prior_request_id.clone(),
                        prior_conversation_id.clone(),
                        effective_attempt_count,
                        recovery_attempted,
                    );
                    if effective_attempt_count > 0 {
                        trace.replacement_request_id =
                            Some(provider_recovery_attempt_request_id(&prepared.job_id, 1));
                        trace.replacement_conversation_id = provider_error
                            .current_url
                            .as_deref()
                            .and_then(conversation_id_from_url);
                    }
                    let packet = build_provider_generation_recovery_packet(
                        ProviderGenerationRecoveryPacketInput {
                            original_input: &prepared.command.prompt,
                            mode: prepared.command.mode,
                            model_class: prepared.command.model_class,
                            effort: prepared.command.effort,
                            files: &prepared.command.files,
                            context_summary: context_summary.clone(),
                            context_manifest: prior_manifest.clone(),
                            recovery_manifest_path: recovery_manifest_path.clone(),
                            replay_files: replay_files.clone(),
                            prior_request_id,
                            prior_conversation_id: prior_conversation_id.clone(),
                            recovery_attempt_count: effective_attempt_count,
                        },
                    );
                    recovery_trace = Some(trace.clone());
                    recovery_packet = Some(packet.clone());

                    let failed_at = Utc::now();
                    db.record_job_failure(
                        &prepared.job_id,
                        failed_at,
                        &provider_error.message,
                        provider_error.current_url.as_deref(),
                    )?;
                    db.update_job_output(
                        &prepared.job_id,
                        None,
                        provider_error.current_url.as_deref(),
                        None,
                    )?;
                    db.update_job_recovery_metadata(&prepared.job_id, Some(&trace), Some(&packet))?;

                    if !recovery_allowed {
                        break Err(execution_error(provider_error.with_recovery(trace, packet)));
                    }

                    for message in [
                        "Provider generation failed. Page Retry disabled.",
                        "Reloading tab and starting a fresh conversation...",
                        "Replaying request with preserved context snapshot...",
                    ] {
                        let updated_at = Utc::now();
                        db.insert_job(
                            &prepared.job_id,
                            JobStatus::Running,
                            Some(message),
                            prior_conversation_id
                                .as_deref()
                                .or(prepared.command.conversation_id.as_deref()),
                            prepared.created_at,
                            updated_at,
                        )?;
                        let mut sink = event_sink.borrow_mut();
                        (*sink)(EventPayload::JobState(JobState {
                            job_id: prepared.job_id.clone(),
                            status: JobStatus::Running,
                            message: Some(message.to_owned()),
                            conversation_id: prior_conversation_id.clone(),
                            updated_at,
                            artifacts: vec![],
                        }))?;
                    }

                    if let Err(reload_err) = lock_browser(&self.browser)?.reload_target_page() {
                        let message = format!("{reload_err:#}");
                        break Err(execution_error(
                            crate::browser::ExecutionErrorMetadata::new(
                                error_code_for_message(&message),
                                message,
                                None,
                            )
                            .with_recovery(trace, packet),
                        ));
                    }

                    recovery_attempt_count = effective_attempt_count;
                    current_command = AskCommand {
                        prompt: build_provider_generation_replay_prompt(
                            &prepared.command.prompt,
                            context_summary.as_deref(),
                            recovery_manifest_path
                                .as_deref()
                                .and_then(|path| std::path::Path::new(path).file_name())
                                .and_then(|value| value.to_str()),
                        ),
                        conversation_id: None,
                        new_conversation: true,
                        mode: prepared.command.mode,
                        model_class: prepared.command.model_class,
                        effort: prepared.command.effort,
                        files: replay_files,
                        auto_recover_provider_generation_failure: false,
                    };
                }
            }
        };

        let _ = lock_cancel_registry(&self.cancel_registry).map(|mut set| {
            set.remove(&prepared.job_id);
        });
        let _ = db.set_job_cancel_requested(&prepared.job_id, false);

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
                for artifact in &result.artifacts {
                    db.insert_artifact(artifact, completed_at)?;
                    db.link_job_artifact(&prepared.job_id, &artifact.artifact_id)?;
                    on_event(EventPayload::ArtifactReady(artifact.clone()))?;
                }
                db.prune_artifacts(ARTIFACT_RETENTION_LIMIT)?;
                let final_status = if recovery_state.recovered || provider_recovered {
                    JobStatus::Recovered
                } else {
                    JobStatus::Completed
                };
                let final_message = if recovery_state.recovered || provider_recovered {
                    "ask recovered"
                } else {
                    "ask completed"
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
                    existing_manifest.as_ref().or_else(|| {
                        result
                            .recovery_packet
                            .as_ref()
                            .and_then(|packet| packet.context_manifest.as_ref())
                    }),
                    &result.conversation_id,
                    &prepared.command.prompt,
                    &result.output_text,
                    &prepared.command.files,
                    &result.artifacts,
                    completed_at,
                );
                db.upsert_conversation(
                    &result.conversation_id,
                    &session_id,
                    "Browser conversation",
                    Some(&result.current_url),
                    Some(&prepared.command.prompt),
                    Some(&result.output_text),
                    Some(&conversation_manifest),
                    completed_at,
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
                let failed_at = Utc::now();
                let typed_error = execution_error_metadata(&err);
                let message = format!("{err:#}");
                let status = if is_cancelled_message(&message) {
                    JobStatus::Cancelled
                } else {
                    JobStatus::Failed
                };
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
                db.insert_job(
                    &prepared.job_id,
                    status,
                    Some(&message),
                    failure_conversation_id.as_deref(),
                    prepared.created_at,
                    failed_at,
                )?;
                db.record_job_failure(&prepared.job_id, failed_at, &message, current_url)?;
                db.update_job_output(&prepared.job_id, None, current_url, None)?;
                if let Some(typed_error) = typed_error {
                    db.update_job_recovery_metadata(
                        &prepared.job_id,
                        typed_error.recovery_trace.as_ref(),
                        typed_error.recovery_packet.as_ref(),
                    )?;
                }
                on_event(EventPayload::JobState(JobState {
                    job_id: prepared.job_id.clone(),
                    status,
                    message: Some(message),
                    conversation_id: failure_conversation_id,
                    updated_at: failed_at,
                    artifacts: vec![],
                }))?;
                Err(err)
            }
        }
    }
}
