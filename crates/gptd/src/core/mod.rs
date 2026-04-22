mod operations;
mod support;

use crate::browser::{
    BrowserManager, execution_error, execution_error_metadata,
};
use crate::config::AppConfig;
use crate::db::Database;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use shared_protocol::{
    AskCommand, AskResult, BrowserOpenRequest, ErrorCode, EventPayload, JobKind, JobState,
    JobStatus, ResearchCommand, ResearchResult,
};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
pub use support::error_code_for_message;
use support::{
    apply_research_update, browser_session_identity, build_provider_generation_context_summary,
    build_provider_generation_recovery_packet, build_provider_generation_recovery_trace,
    build_provider_generation_replay_prompt, conversation_id_from_url, is_cancelled_message,
    lock_browser, lock_cancel_registry, maybe_write_research_output,
    provider_recovery_attempt_request_id, should_auto_recover_provider_generation_failure,
};

const ARTIFACT_RETENTION_LIMIT: usize = 100;

#[derive(Debug, Default)]
struct ExecutionRecoveryState {
    interrupted: bool,
    recovered: bool,
}

#[derive(Clone)]
pub struct CoreService {
    config: AppConfig,
    started_at: chrono::DateTime<Utc>,
    browser: Arc<Mutex<BrowserManager>>,
    cancel_registry: Arc<Mutex<HashSet<String>>>,
}

#[derive(Debug, Clone)]
pub struct PreparedAskJob {
    pub job_id: String,
    pub command: AskCommand,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct PreparedResearchJob {
    pub job_id: String,
    pub command: ResearchCommand,
    pub created_at: DateTime<Utc>,
}

impl CoreService {
    pub fn new(config: AppConfig) -> Result<Self> {
        let db = Database::open(&config.paths.db_path)?;
        db.fail_incomplete_jobs("daemon restarted before the job finished", Utc::now())?;

        Ok(Self {
            config: config.clone(),
            started_at: Utc::now(),
            browser: Arc::new(Mutex::new(BrowserManager::new(&config))),
            cancel_registry: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn warm_browser_if_enabled(&self) -> Result<()> {
        if self.config.warm_mode() {
            lock_browser(&self.browser)?.open(BrowserOpenRequest {
                target_url: None,
                browser_path: None,
                headless: None,
            })?;
        }
        Ok(())
    }

    pub fn ask<F>(&self, command: AskCommand, mut on_event: F) -> Result<AskResult>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let prepared = self.prepare_ask(command)?;
        on_event(EventPayload::JobState(JobState {
            job_id: prepared.job_id.clone(),
            status: JobStatus::Queued,
            message: Some("queued ask job".to_owned()),
            conversation_id: prepared.command.conversation_id.clone(),
            updated_at: prepared.created_at,
            artifacts: vec![],
        }))?;
        self.execute_prepared_ask(&prepared, on_event)
    }

    pub fn research<F>(&self, command: ResearchCommand, mut on_event: F) -> Result<ResearchResult>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let prepared = self.prepare_research(command)?;
        on_event(EventPayload::JobState(JobState {
            job_id: prepared.job_id.clone(),
            status: JobStatus::Queued,
            message: Some("queued research job".to_owned()),
            conversation_id: prepared.command.conversation_id.clone(),
            updated_at: prepared.created_at,
            artifacts: vec![],
        }))?;
        self.execute_prepared_research(&prepared, on_event)
    }

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
                    let prior_conversation = if let Some(conversation_id) =
                        prior_conversation_id.as_deref()
                    {
                        db.find_conversation(conversation_id)?
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
                        prior_conversation.as_ref(),
                        prior_conversation_id.as_deref(),
                    );
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
                        &prepared.command.prompt,
                        prepared.command.mode,
                        prepared.command.model_class,
                        prepared.command.effort,
                        &prepared.command.files,
                        context_summary.clone(),
                        prior_request_id,
                        prior_conversation_id.clone(),
                        effective_attempt_count,
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
                    db.update_job_recovery_metadata(
                        &prepared.job_id,
                        Some(&trace),
                        Some(&packet),
                    )?;

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
                        ),
                        conversation_id: None,
                        new_conversation: true,
                        mode: prepared.command.mode,
                        model_class: prepared.command.model_class,
                        effort: prepared.command.effort,
                        files: prepared.command.files.clone(),
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
                db.upsert_conversation(
                    &result.conversation_id,
                    &session_id,
                    "Browser conversation",
                    Some(&result.current_url),
                    Some(&prepared.command.prompt),
                    Some(&result.output_text),
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
                                error.current_url.as_deref().and_then(conversation_id_from_url)
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
                db.upsert_conversation(
                    &result.conversation_id,
                    &session_id,
                    "Browser conversation",
                    Some(&result.current_url),
                    Some(&prepared.command.query),
                    Some(&result.report_text),
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
