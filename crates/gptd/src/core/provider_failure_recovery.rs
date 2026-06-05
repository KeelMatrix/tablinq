use super::ask::PreparedAskJob;
use super::lifecycle::record_failure_checkpoint;
use super::recovery::{
    ProviderGenerationRecoveryPacketInput, build_provider_generation_context_summary,
    build_provider_generation_recovery_packet, build_provider_generation_recovery_trace,
    build_provider_generation_replay_files, build_provider_generation_replay_prompt,
    provider_recovery_attempt_request_id, should_auto_recover_provider_generation_failure,
};
use super::recovery_manifest::write_provider_generation_recovery_manifest_attachment;
use super::support::{conversation_id_from_url, lock_browser};
use super::{CoreService, error_code_for_message};
use crate::browser::{execution_error, execution_error_metadata};
use crate::db::Database;
use anyhow::{Error, Result};
use chrono::Utc;
use shared_protocol::{
    AskCommand, ErrorCode, EventPayload, JobState, JobStatus, RecoveryPacket, RecoveryTrace,
};

pub(super) struct ProviderFailureRecoveryInput<'a> {
    pub db: &'a Database,
    pub prepared: &'a PreparedAskJob,
    pub current_command: &'a AskCommand,
    pub latest_update: Option<crate::browser::RequestLifecycleUpdate>,
    pub err: Error,
    pub recovery_trace: Option<&'a RecoveryTrace>,
    pub recovery_packet: Option<&'a RecoveryPacket>,
    pub recovery_attempt_count: u32,
}

pub(super) enum ProviderFailureRecoveryOutcome {
    Replay {
        trace: Box<RecoveryTrace>,
        packet: Box<RecoveryPacket>,
        next_command: AskCommand,
        attempt_count: u32,
    },
    TerminalError(Error),
}

impl CoreService {
    pub(super) fn handle_provider_failure_recovery<F>(
        &self,
        input: ProviderFailureRecoveryInput<'_>,
        on_event: &mut F,
    ) -> Result<ProviderFailureRecoveryOutcome>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let current_url = input
            .latest_update
            .as_ref()
            .and_then(|update| update.current_url.clone());
        let typed_error = execution_error_metadata(&input.err);
        if !matches!(
            typed_error.as_ref().map(|error| error.error_code),
            Some(ErrorCode::ProviderGenerationFailedNonretryable)
        ) {
            return Ok(ProviderFailureRecoveryOutcome::TerminalError(
                attach_existing_recovery_metadata(
                    input.err,
                    current_url,
                    input.recovery_trace,
                    input.recovery_packet,
                ),
            ));
        }

        let provider_error = typed_error.expect("provider failure metadata missing");
        let prior_conversation_id = input
            .latest_update
            .as_ref()
            .and_then(|update| update.conversation_id.clone())
            .or_else(|| input.current_command.conversation_id.clone())
            .or_else(|| {
                provider_error
                    .current_url
                    .as_deref()
                    .and_then(conversation_id_from_url)
            });
        let prior_conversation = if let Some(conversation_id) = prior_conversation_id.as_deref() {
            input.db.find_conversation(conversation_id)?
        } else {
            None
        };
        let prior_manifest = if let Some(conversation_id) = prior_conversation_id.as_deref() {
            input.db.conversation_recovery_manifest(conversation_id)?
        } else {
            None
        };
        let recovery_allowed = should_auto_recover_provider_generation_failure(
            input
                .prepared
                .command
                .auto_recover_provider_generation_failure,
            input.recovery_attempt_count,
        );
        let recovery_attempted = input.recovery_attempt_count > 0 || recovery_allowed;
        let effective_attempt_count = if recovery_allowed {
            input.recovery_attempt_count + 1
        } else {
            input.recovery_attempt_count
        };
        let prior_request_id = Some(provider_recovery_attempt_request_id(
            &input.prepared.job_id,
            0,
        ));
        let failure_bundle_id = if let Some(trace) = input.recovery_trace {
            trace.failure_bundle_id.clone()
        } else {
            self.capture_failure_bundle(
                Some(input.prepared.command.mode),
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
        let previous_recovery_packet = input.recovery_packet.cloned();
        let recovery_manifest_path = if recovery_allowed {
            prior_manifest
                .as_ref()
                .map(|manifest| {
                    write_provider_generation_recovery_manifest_attachment(
                        &self.config.paths.state_root,
                        &input.prepared.job_id,
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
                &input.prepared.command.files,
                prior_manifest.as_ref(),
                recovery_manifest_path.as_deref(),
            )
        } else {
            previous_recovery_packet
                .as_ref()
                .map(|packet| packet.replay_files.clone())
                .unwrap_or_else(|| input.prepared.command.files.clone())
        };
        let mut trace = build_provider_generation_recovery_trace(
            failure_bundle_id,
            prior_request_id.clone(),
            prior_conversation_id.clone(),
            effective_attempt_count,
            recovery_attempted,
        );
        if effective_attempt_count > 0 {
            trace.replacement_request_id = Some(provider_recovery_attempt_request_id(
                &input.prepared.job_id,
                1,
            ));
            trace.replacement_conversation_id = provider_error
                .current_url
                .as_deref()
                .and_then(conversation_id_from_url);
        }
        let packet =
            build_provider_generation_recovery_packet(ProviderGenerationRecoveryPacketInput {
                original_input: &input.prepared.command.prompt,
                mode: input.prepared.command.mode,
                model_class: input.prepared.command.model_class,
                effort: input.prepared.command.effort,
                files: &input.prepared.command.files,
                context_summary: context_summary.clone(),
                context_manifest: prior_manifest.clone(),
                recovery_manifest_path: recovery_manifest_path.clone(),
                replay_files: replay_files.clone(),
                prior_request_id,
                prior_conversation_id: prior_conversation_id.clone(),
                recovery_attempt_count: effective_attempt_count,
            });

        let failed_at = Utc::now();
        record_failure_checkpoint(
            input.db,
            &input.prepared.job_id,
            failed_at,
            &provider_error.message,
            provider_error.current_url.as_deref(),
            Some(&trace),
            Some(&packet),
        )?;

        if !recovery_allowed {
            return Ok(ProviderFailureRecoveryOutcome::TerminalError(
                execution_error(provider_error.with_recovery(trace, packet)),
            ));
        }

        emit_provider_recovery_progress(
            input.db,
            input.prepared,
            &prior_conversation_id,
            on_event,
        )?;

        if let Err(reload_err) = lock_browser(&self.browser)?.reload_target_page() {
            let message = format!("{reload_err:#}");
            return Ok(ProviderFailureRecoveryOutcome::TerminalError(
                execution_error(
                    crate::browser::ExecutionErrorMetadata::new(
                        error_code_for_message(&message),
                        message,
                        None,
                    )
                    .with_recovery(trace, packet),
                ),
            ));
        }

        let mut next_command = AskCommand {
            prompt: build_provider_generation_replay_prompt(
                &input.prepared.command.prompt,
                context_summary.as_deref(),
                recovery_manifest_path
                    .as_deref()
                    .and_then(|path| std::path::Path::new(path).file_name())
                    .and_then(|value| value.to_str()),
            ),
            conversation_id: None,
            new_conversation: true,
            mode: input.prepared.command.mode,
            model_class: input.prepared.command.model_class,
            effort: input.prepared.command.effort,
            files: replay_files,
            prompt_upload_threshold_chars: None,
            auto_recover_provider_generation_failure: false,
        };
        if let Err(err) = self.apply_native_upload_budget_to_ask(&mut next_command) {
            return Ok(ProviderFailureRecoveryOutcome::TerminalError(err));
        }

        Ok(ProviderFailureRecoveryOutcome::Replay {
            trace: Box::new(trace),
            packet: Box::new(packet),
            next_command,
            attempt_count: effective_attempt_count,
        })
    }
}

fn attach_existing_recovery_metadata(
    err: Error,
    current_url: Option<String>,
    recovery_trace: Option<&RecoveryTrace>,
    recovery_packet: Option<&RecoveryPacket>,
) -> Error {
    if let (Some(trace), Some(packet)) = (recovery_trace.cloned(), recovery_packet.cloned()) {
        let message = format!("{err:#}");
        execution_error(
            crate::browser::ExecutionErrorMetadata::new(
                error_code_for_message(&message),
                message,
                current_url,
            )
            .with_recovery(trace, packet),
        )
    } else {
        err
    }
}

fn emit_provider_recovery_progress<F>(
    db: &Database,
    prepared: &PreparedAskJob,
    prior_conversation_id: &Option<String>,
    on_event: &mut F,
) -> Result<()>
where
    F: FnMut(EventPayload) -> Result<()>,
{
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
        on_event(EventPayload::JobState(JobState {
            job_id: prepared.job_id.clone(),
            status: JobStatus::Running,
            message: Some(message.to_owned()),
            conversation_id: prior_conversation_id.clone(),
            updated_at,
            artifacts: vec![],
        }))?;
    }

    Ok(())
}
