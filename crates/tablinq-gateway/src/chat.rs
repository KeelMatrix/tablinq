use crate::openai::{self, ChatCompletionsRequest, RequestRole, request_role_name, resolve_model};
use crate::uploads::UploadStore;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use gptd::browser::execution_error_metadata;
use gptd::CoreService;
use gptd::core::{PreparedAskJob, PreparedResearchJob};
use gptd::logging::JsonLogger;
use serde_json::{Value, json};
use shared_protocol::{
    EventPayload, JobStatus, RecoveryPacket, RecoveryTrace, RequestExecutionKind, RequestSummary,
    RequestUsage, StreamChunkKind,
};
use std::net::TcpStream;

use super::{
    HttpResponse, RequestContext, capture_gateway_failure_bundle, ensure_browser_session,
    is_client_disconnect_error, json_response, log_response, map_openai_error_value,
    write_http_response, write_sse_done, write_sse_event, write_sse_headers,
};

pub(crate) enum PreparedChatJob {
    Ask(PreparedAskJob),
    Research(PreparedResearchJob),
}

pub(crate) struct PreparedChatRequest {
    pub(crate) request: ChatCompletionsRequest,
    pub(crate) model_id: &'static str,
    pub(crate) request_role: RequestRole,
    pub(crate) parent_request_id: Option<String>,
    pub(crate) file_ids: Vec<String>,
    pub(crate) job: PreparedChatJob,
}

impl PreparedChatRequest {
    pub(crate) fn job_id(&self) -> &str {
        match &self.job {
            PreparedChatJob::Ask(job) => &job.job_id,
            PreparedChatJob::Research(job) => &job.job_id,
        }
    }

    pub(crate) fn mode(&self) -> shared_protocol::Mode {
        match &self.job {
            PreparedChatJob::Ask(job) => job.command.mode,
            PreparedChatJob::Research(_) => shared_protocol::Mode::Research,
        }
    }

    pub(crate) fn conversation_id_hint(&self) -> Option<&str> {
        match &self.job {
            PreparedChatJob::Ask(job) => job.command.conversation_id.as_deref(),
            PreparedChatJob::Research(job) => job.command.conversation_id.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestOutcomeKind {
    Completed,
    Recovered,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone)]
pub(crate) struct RequestOutcome {
    pub(crate) kind: RequestOutcomeKind,
    pub(crate) low_confidence: bool,
    pub(crate) conversation_id: Option<String>,
    pub(crate) interrupted_at: Option<DateTime<Utc>>,
    pub(crate) interruption_reason: Option<String>,
    pub(crate) recovered_at: Option<DateTime<Utc>>,
    pub(crate) recovery_reason: Option<String>,
    pub(crate) failed_at: Option<DateTime<Utc>>,
    pub(crate) failure_reason: Option<String>,
    pub(crate) recovery_trace: Option<RecoveryTrace>,
    pub(crate) recovery_packet: Option<RecoveryPacket>,
}

pub(crate) fn prepare_chat_completions_request(
    core: &CoreService,
    upload_store: &UploadStore,
    context: &RequestContext,
) -> Result<PreparedChatRequest> {
    let request: ChatCompletionsRequest =
        serde_json::from_slice(&context.request.body).context("decode chat completions request")?;
    openai::validate_chat_request(&request)?;
    let model = resolve_model(&request.model).context("resolve provider model")?;
    let request_role = request.request_role();
    let parent_request_id = request.tablinq_parent_request_id.clone();
    let flattened = openai::flatten_messages(&request)?;
    let files = upload_store.resolve_file_inputs(flattened.file_paths, &flattened.file_ids)?;
    let conversation_id = request
        .tablinq_conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let new_conversation = request.tablinq_new_conversation;

    let job = if model.uses_research {
        PreparedChatJob::Research(core.prepare_research(shared_protocol::ResearchCommand {
            query: flattened.prompt,
            conversation_id,
            new_conversation,
            files,
            output_dir: None,
        })?)
    } else {
        PreparedChatJob::Ask(core.prepare_ask(shared_protocol::AskCommand {
            prompt: flattened.prompt,
            conversation_id,
            new_conversation,
            mode: model.mode,
            model_class: model.model_class,
            effort: model.effort,
            files,
            auto_recover_provider_generation_failure:
                request.tablinq_auto_recover_provider_generation_failure,
        })?)
    };

    Ok(PreparedChatRequest {
        request,
        model_id: model.id,
        request_role,
        parent_request_id,
        file_ids: flattened.file_ids,
        job,
    })
}

pub(crate) fn execute_chat_completions_request(
    stream: &mut TcpStream,
    core: &CoreService,
    context: &RequestContext,
    logger: &JsonLogger,
    prepared: &PreparedChatRequest,
    client_id: &str,
) -> Result<RequestOutcome> {
    ensure_browser_session(core)?;

    if prepared.request.stream {
        let stream_id = format!("chatcmpl-{}", Utc::now().timestamp_millis());
        let mut downstream_interrupted: Option<(DateTime<Utc>, String)> = None;
        if let Err(err) = write_sse_headers(stream, &context.request_id) {
            if is_client_disconnect_error(&err) {
                let interrupted_at = Utc::now();
                let message =
                    format!("gateway client disconnected before streaming completed: {err:#}");
                let _ = core.mark_job_interrupted(
                    prepared.job_id(),
                    prepared.conversation_id_hint(),
                    &message,
                );
                downstream_interrupted = Some((interrupted_at, message));
            } else {
                return Err(err);
            }
        }
        if downstream_interrupted.is_none()
            && let Err(err) = write_sse_event(
                stream,
                &json!({
                    "id": stream_id,
                    "object": "chat.completion.chunk",
                    "created": Utc::now().timestamp(),
                    "model": prepared.model_id,
                    "request_id": context.request_id,
                    "tablinq_job_id": prepared.job_id(),
                    "tablinq_request_role": request_role_name(prepared.request_role),
                    "tablinq_parent_request_id": prepared.parent_request_id.as_deref(),
                    "choices": [{
                        "index": 0,
                        "delta": { "role": "assistant" },
                        "finish_reason": Value::Null,
                    }],
                }),
            )
        {
            if is_client_disconnect_error(&err) {
                let interrupted_at = Utc::now();
                let message =
                    format!("gateway client disconnected before streaming completed: {err:#}");
                let _ = core.mark_job_interrupted(
                    prepared.job_id(),
                    prepared.conversation_id_hint(),
                    &message,
                );
                downstream_interrupted = Some((interrupted_at, message));
            } else {
                return Err(err);
            }
        }

        let result = match &prepared.job {
            PreparedChatJob::Research(job) => core
                .execute_prepared_research(job, |event| {
                    if downstream_interrupted.is_some() {
                        return Ok(());
                    }
                    if let Err(err) = write_stream_delta(
                        stream,
                        &stream_id,
                        prepared.model_id,
                        &context.request_id,
                        event.clone(),
                    ) {
                        if is_client_disconnect_error(&err) {
                            let interrupted_at = Utc::now();
                            let message = format!(
                                "gateway client disconnected before streaming completed: {err:#}"
                            );
                            let _ = core.mark_job_interrupted(
                                prepared.job_id(),
                                event_conversation_id(&event).as_deref(),
                                &message,
                            );
                            downstream_interrupted = Some((interrupted_at, message));
                            return Ok(());
                        }
                        return Err(err);
                    }
                    Ok(())
                })
                .map(|result| {
                    (
                        result.conversation_id,
                        result.low_confidence,
                        None,
                        None,
                    )
                }),
            PreparedChatJob::Ask(job) => core
                .execute_prepared_ask(job, |event| {
                    if downstream_interrupted.is_some() {
                        return Ok(());
                    }
                    if let Err(err) = write_stream_delta(
                        stream,
                        &stream_id,
                        prepared.model_id,
                        &context.request_id,
                        event.clone(),
                    ) {
                        if is_client_disconnect_error(&err) {
                            let interrupted_at = Utc::now();
                            let message = format!(
                                "gateway client disconnected before streaming completed: {err:#}"
                            );
                            let _ = core.mark_job_interrupted(
                                prepared.job_id(),
                                event_conversation_id(&event).as_deref(),
                                &message,
                            );
                            downstream_interrupted = Some((interrupted_at, message));
                            return Ok(());
                        }
                        return Err(err);
                    }
                    Ok(())
                })
                .map(|result| {
                    (
                        result.conversation_id,
                        result.low_confidence,
                        result.recovery_trace,
                        result.recovery_packet,
                    )
                }),
        };

        match result {
            Ok((conversation_id, low_confidence, recovery_trace, recovery_packet)) => {
                let recovered_at = recovery_trace
                    .as_ref()
                    .filter(|trace| trace.recovery_attempted)
                    .map(|_| Utc::now());
                let recovery_reason = recovery_trace
                    .as_ref()
                    .filter(|trace| trace.recovery_attempted)
                    .and_then(recovery_reason_for_trace);
                if let Some((interrupted_at, interruption_reason)) = downstream_interrupted.take() {
                    let downstream_recovered_at = Utc::now();
                    let recovery_reason =
                        "gateway client disconnected, but the upstream request completed"
                            .to_owned();
                    let _ = core.mark_job_recovered(
                        prepared.job_id(),
                        Some(&conversation_id),
                        &recovery_reason,
                    );
                    logger.info(
                        "gateway recovered interrupted client stream",
                        json!({
                            "request_id": context.request_id,
                            "job_id": prepared.job_id(),
                        }),
                    )?;
                    return Ok(RequestOutcome {
                        kind: RequestOutcomeKind::Recovered,
                        low_confidence: true,
                        conversation_id: Some(conversation_id),
                        interrupted_at: Some(interrupted_at),
                        interruption_reason: Some(interruption_reason),
                        recovered_at: Some(downstream_recovered_at),
                        recovery_reason: Some(recovery_reason),
                        failed_at: None,
                        failure_reason: None,
                        recovery_trace,
                        recovery_packet,
                    });
                }

                if let Err(err) = write_sse_event(
                    stream,
                    &json!({
                        "id": stream_id,
                        "object": "chat.completion.chunk",
                        "created": Utc::now().timestamp(),
                        "model": prepared.model_id,
                        "request_id": context.request_id,
                        "tablinq_job_id": prepared.job_id(),
                        "tablinq_request_role": request_role_name(prepared.request_role),
                        "tablinq_parent_request_id": prepared.parent_request_id.as_deref(),
                        "choices": [{
                            "index": 0,
                            "delta": {},
                            "finish_reason": "stop",
                        }],
                    }),
                ) {
                    if is_client_disconnect_error(&err) {
                        let interrupted_at = Utc::now();
                        let interruption_reason = format!(
                            "gateway client disconnected before streaming completed: {err:#}"
                        );
                        let recovered_at = Utc::now();
                        let recovery_reason =
                            "gateway client disconnected, but the upstream request completed"
                                .to_owned();
                        let _ = core.mark_job_interrupted(
                            prepared.job_id(),
                            Some(&conversation_id),
                            &interruption_reason,
                        );
                        let _ = core.mark_job_recovered(
                            prepared.job_id(),
                            Some(&conversation_id),
                            &recovery_reason,
                        );
                        return Ok(RequestOutcome {
                            kind: RequestOutcomeKind::Recovered,
                            low_confidence: true,
                            conversation_id: Some(conversation_id),
                            interrupted_at: Some(interrupted_at),
                            interruption_reason: Some(interruption_reason),
                            recovered_at: Some(recovered_at),
                            recovery_reason: Some(recovery_reason),
                            failed_at: None,
                            failure_reason: None,
                            recovery_trace: recovery_trace.clone(),
                            recovery_packet: recovery_packet.clone(),
                        });
                    }
                    return Err(err);
                }
                if let Err(err) = write_sse_done(stream) {
                    if is_client_disconnect_error(&err) {
                        let interrupted_at = Utc::now();
                        let interruption_reason = format!(
                            "gateway client disconnected before streaming completed: {err:#}"
                        );
                        let recovered_at = Utc::now();
                        let recovery_reason =
                            "gateway client disconnected, but the upstream request completed"
                                .to_owned();
                        let _ = core.mark_job_interrupted(
                            prepared.job_id(),
                            Some(&conversation_id),
                            &interruption_reason,
                        );
                        let _ = core.mark_job_recovered(
                            prepared.job_id(),
                            Some(&conversation_id),
                            &recovery_reason,
                        );
                        return Ok(RequestOutcome {
                            kind: RequestOutcomeKind::Recovered,
                            low_confidence: true,
                            conversation_id: Some(conversation_id),
                            interrupted_at: Some(interrupted_at),
                            interruption_reason: Some(interruption_reason),
                            recovered_at: Some(recovered_at),
                            recovery_reason: Some(recovery_reason),
                            failed_at: None,
                            failure_reason: None,
                            recovery_trace: recovery_trace.clone(),
                            recovery_packet: recovery_packet.clone(),
                        });
                    }
                    return Err(err);
                }
                logger.info(
                    "gateway streamed chat completion",
                    json!({
                        "request_id": context.request_id,
                        "model": prepared.model_id,
                        "job_id": prepared.job_id(),
                    }),
                )?;
                Ok(RequestOutcome {
                    kind: if low_confidence {
                        RequestOutcomeKind::Recovered
                    } else {
                        RequestOutcomeKind::Completed
                    },
                    low_confidence,
                    conversation_id: Some(conversation_id),
                    interrupted_at: None,
                    interruption_reason: None,
                    recovered_at,
                    recovery_reason,
                    failed_at: None,
                    failure_reason: None,
                    recovery_trace,
                    recovery_packet,
                })
            }
            Err(err) => {
                let failure_bundle_id =
                    capture_gateway_failure_bundle(core, context, prepared, client_id, &err);
                if downstream_interrupted.is_none() {
                    write_sse_event(
                        stream,
                        &map_openai_error_value(
                            &context.request_id,
                            &err,
                            failure_bundle_id.as_deref(),
                        ),
                    )?;
                    write_sse_done(stream)?;
                }
                let mut outcome = outcome_from_error(&err);
                if let Some((interrupted_at, interruption_reason)) = downstream_interrupted {
                    outcome.interrupted_at = Some(interrupted_at);
                    outcome.interruption_reason = Some(interruption_reason);
                    outcome.low_confidence = true;
                }
                Ok(outcome)
            }
        }
    } else {
        let (response, conversation_id, low_confidence, recovery_trace, recovery_packet) =
            match &prepared.job {
            PreparedChatJob::Research(job) => {
                let result = core.execute_prepared_research(job, |_event| Ok(()))?;
                let conversation = core.find_conversation(&result.conversation_id)?;
                (
                    build_chat_completion_response(
                        &context.request_id,
                        prepared.job_id(),
                        prepared.model_id,
                        prepared.request_role,
                        prepared.parent_request_id.as_deref(),
                        Some(&result.conversation_id),
                        conversation.as_ref().map(|item| item.session_id.as_str()),
                        &result.report_text,
                        "stop",
                        None,
                    ),
                    Some(result.conversation_id),
                    result.low_confidence,
                    None,
                    None,
                )
            }
            PreparedChatJob::Ask(job) => {
                let result = core.execute_prepared_ask(job, |_event| Ok(()))?;
                let conversation = core.find_conversation(&result.conversation_id)?;
                (
                    build_chat_completion_response(
                        &context.request_id,
                        prepared.job_id(),
                        prepared.model_id,
                        prepared.request_role,
                        prepared.parent_request_id.as_deref(),
                        Some(&result.conversation_id),
                        conversation.as_ref().map(|item| item.session_id.as_str()),
                        &result.output_text,
                        "stop",
                        result.recovery_trace.as_ref(),
                    ),
                    Some(result.conversation_id),
                    result.low_confidence,
                    result.recovery_trace,
                    result.recovery_packet,
                )
            }
        };
        log_response(logger, &context.request_id, response.status_code)?;
        if let Err(err) = write_http_response(stream, &context.request_id, response) {
            if is_client_disconnect_error(&err) {
                let interrupted_at = Utc::now();
                let interruption_reason = format!(
                    "gateway client disconnected before the final response was written: {err:#}"
                );
                let recovered_at = Utc::now();
                let recovery_reason =
                    "gateway client disconnected, but the upstream request completed".to_owned();
                let _ = core.mark_job_interrupted(
                    prepared.job_id(),
                    conversation_id.as_deref(),
                    &interruption_reason,
                );
                let _ = core.mark_job_recovered(
                    prepared.job_id(),
                    conversation_id.as_deref(),
                    &recovery_reason,
                );
                return Ok(RequestOutcome {
                    kind: RequestOutcomeKind::Recovered,
                    low_confidence: true,
                    conversation_id,
                    interrupted_at: Some(interrupted_at),
                    interruption_reason: Some(interruption_reason),
                    recovered_at: Some(recovered_at),
                    recovery_reason: Some(recovery_reason),
                    failed_at: None,
                    failure_reason: None,
                    recovery_trace,
                    recovery_packet,
                });
            }
            return Err(err);
        }
        let recovered_at = recovery_trace
            .as_ref()
            .filter(|trace| trace.recovery_attempted)
            .map(|_| Utc::now());
        let recovery_reason = recovery_trace
            .as_ref()
            .filter(|trace| trace.recovery_attempted)
            .and_then(recovery_reason_for_trace);
        Ok(RequestOutcome {
            kind: if low_confidence {
                RequestOutcomeKind::Recovered
            } else {
                RequestOutcomeKind::Completed
            },
            low_confidence,
            conversation_id,
            interrupted_at: None,
            interruption_reason: None,
            recovered_at,
            recovery_reason,
            failed_at: None,
            failure_reason: None,
            recovery_trace,
            recovery_packet,
        })
    }
}

pub(crate) fn build_request_summary(
    context: &RequestContext,
    prepared: &PreparedChatRequest,
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    queue_wait_ms: u64,
    outcome: RequestOutcome,
) -> RequestSummary {
    RequestSummary {
        request_id: context.request_id.clone(),
        job_id: prepared.job_id().to_owned(),
        model_id: Some(prepared.model_id.to_owned()),
        request_role: Some(request_role_name(prepared.request_role).to_owned()),
        parent_request_id: prepared.parent_request_id.clone(),
        execution_kind: match &prepared.job {
            PreparedChatJob::Ask(_) => RequestExecutionKind::Ask,
            PreparedChatJob::Research(_) => RequestExecutionKind::Research,
        },
        status: match outcome.kind {
            RequestOutcomeKind::Completed => JobStatus::Completed,
            RequestOutcomeKind::Recovered => JobStatus::Recovered,
            RequestOutcomeKind::Failed => JobStatus::Failed,
            RequestOutcomeKind::Cancelled => JobStatus::Cancelled,
        },
        cancelled: outcome.kind == RequestOutcomeKind::Cancelled,
        start_time,
        end_time: Some(end_time),
        duration_ms: Some((end_time - start_time).num_milliseconds().max(0) as u64),
        queue_wait_ms,
        interrupted_at: outcome.interrupted_at,
        interruption_reason: outcome.interruption_reason,
        recovered_at: outcome.recovered_at,
        recovery_reason: outcome.recovery_reason,
        failed_at: outcome.failed_at,
        failure_reason: outcome.failure_reason,
        recovery_trace: outcome.recovery_trace,
        recovery_packet: outcome.recovery_packet,
        usage: RequestUsage {
            status: "unknown".to_owned(),
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: None,
        },
        warnings: vec![
            "exact token usage is unavailable for the browser-backed provider".to_owned(),
        ],
    }
}

pub(crate) fn outcome_from_error(err: &anyhow::Error) -> RequestOutcome {
    let metadata = execution_error_metadata(err);
    let message = metadata
        .as_ref()
        .map(|item| item.message.clone())
        .unwrap_or_else(|| format!("{err:#}"));
    RequestOutcome {
        kind: if message.contains("cancelled") {
            RequestOutcomeKind::Cancelled
        } else {
            RequestOutcomeKind::Failed
        },
        low_confidence: message.contains("timed out"),
        conversation_id: None,
        interrupted_at: None,
        interruption_reason: None,
        recovered_at: None,
        recovery_reason: None,
        failed_at: if message.contains("cancelled") {
            None
        } else {
            Some(Utc::now())
        },
        failure_reason: if message.contains("cancelled") {
            None
        } else {
            Some(message)
        },
        recovery_trace: metadata
            .as_ref()
            .and_then(|item| item.recovery_trace.clone()),
        recovery_packet: metadata.and_then(|item| item.recovery_packet),
    }
}

fn event_conversation_id(event: &EventPayload) -> Option<String> {
    match event {
        EventPayload::StreamChunk(chunk) => chunk.conversation_id.clone(),
        EventPayload::JobState(state) => state.conversation_id.clone(),
        EventPayload::ArtifactReady(artifact) => artifact.conversation_id.clone(),
        _ => None,
    }
}

fn write_stream_delta(
    stream: &mut TcpStream,
    stream_id: &str,
    model_id: &str,
    request_id: &str,
    event: EventPayload,
) -> Result<()> {
    if let EventPayload::StreamChunk(chunk) = event
        && chunk.kind == StreamChunkKind::Delta
        && !chunk.text.is_empty()
    {
        write_sse_event(
            stream,
            &json!({
                "id": stream_id,
                "object": "chat.completion.chunk",
                "created": Utc::now().timestamp(),
                "model": model_id,
                "request_id": request_id,
                "tablinq_conversation_id": chunk.conversation_id,
                "choices": [{
                    "index": 0,
                    "delta": { "content": chunk.text },
                    "finish_reason": Value::Null,
                }],
            }),
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_chat_completion_response(
    request_id: &str,
    job_id: &str,
    model_id: &str,
    request_role: RequestRole,
    parent_request_id: Option<&str>,
    conversation_id: Option<&str>,
    session_id: Option<&str>,
    content: &str,
    finish_reason: &str,
    recovery_trace: Option<&RecoveryTrace>,
) -> HttpResponse {
    json_response(
        200,
        json!({
            "id": format!("chatcmpl-{}", Utc::now().timestamp_millis()),
            "object": "chat.completion",
            "created": Utc::now().timestamp(),
            "model": model_id,
            "request_id": request_id,
            "tablinq_job_id": job_id,
            "tablinq_request_role": request_role_name(request_role),
            "tablinq_parent_request_id": parent_request_id,
            "tablinq_conversation_id": conversation_id,
            "tablinq_session_id": session_id,
            "tablinq_recovery_available": recovery_trace.map(|trace| trace.recovery_available).unwrap_or(false),
            "tablinq_recovery_attempted": recovery_trace.map(|trace| trace.recovery_attempted).unwrap_or(false),
            "tablinq_recovery_strategy": recovery_trace.and_then(|trace| trace.recovery_strategy.as_ref()).map(recovery_strategy_name),
            "tablinq_prior_request_id": recovery_trace.and_then(|trace| trace.prior_request_id.as_deref()),
            "tablinq_replacement_request_id": recovery_trace.and_then(|trace| trace.replacement_request_id.as_deref()),
            "tablinq_replacement_conversation_id": recovery_trace.and_then(|trace| trace.replacement_conversation_id.as_deref()),
            "tablinq_failure_bundle_id": recovery_trace.and_then(|trace| trace.failure_bundle_id.as_deref()),
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": content,
                },
                "finish_reason": finish_reason,
            }],
            "usage": Value::Null,
            "warnings": ["usage is unavailable for the browser-backed provider"],
        }),
    )
}

fn recovery_strategy_name(strategy: &shared_protocol::RecoveryStrategy) -> &'static str {
    match strategy {
        shared_protocol::RecoveryStrategy::ReloadNewConversationReplayWithContext => {
            "reload_new_conversation_replay_with_context"
        }
    }
}

fn recovery_reason_for_trace(trace: &RecoveryTrace) -> Option<String> {
    trace
        .recovery_strategy
        .as_ref()
        .map(recovery_strategy_name)
        .map(|strategy| format!("provider generation failure recovered via {strategy}"))
}
