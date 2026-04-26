use super::interruption::{
    DownstreamInterruption, DownstreamRecovery, final_response_disconnect_message,
    mark_downstream_interrupted, recover_downstream_interruption, stream_disconnect_message,
};
use super::preparation::{PreparedChatJob, PreparedChatRequest};
use super::response::{build_chat_completion_response, recovery_reason_for_trace};
use super::sse_writer::{
    write_stream_delta, write_stream_done, write_stream_error, write_stream_finish_chunk,
    write_stream_headers, write_stream_role_chunk,
};
use super::{RequestOutcome, RequestOutcomeKind, event_conversation_id, outcome_from_error};
use crate::browser::ensure_browser_session;
use crate::errors::{
    capture_gateway_failure_bundle, is_client_disconnect_error, map_openai_error_value,
};
use crate::http::{HttpResponse, RequestContext, log_response, write_http_response};
use anyhow::Result;
use chrono::Utc;
use gptd::CoreService;
use gptd::logging::JsonLogger;
use shared_protocol::{EventPayload, RecoveryPacket, RecoveryTrace};
use std::net::TcpStream;

struct ChatExecutionResult {
    conversation_id: String,
    low_confidence: bool,
    recovery_trace: Option<RecoveryTrace>,
    recovery_packet: Option<RecoveryPacket>,
}

struct NonStreamingExecution {
    response: HttpResponse,
    conversation_id: Option<String>,
    low_confidence: bool,
    recovery_trace: Option<RecoveryTrace>,
    recovery_packet: Option<RecoveryPacket>,
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
        execute_streaming_chat_completion(stream, core, context, logger, prepared, client_id)
    } else {
        execute_non_streaming_chat_completion(stream, core, context, logger, prepared)
    }
}

fn execute_streaming_chat_completion(
    stream: &mut TcpStream,
    core: &CoreService,
    context: &RequestContext,
    logger: &JsonLogger,
    prepared: &PreparedChatRequest,
    client_id: &str,
) -> Result<RequestOutcome> {
    let stream_id = format!("chatcmpl-{}", Utc::now().timestamp_millis());
    let mut downstream_interruption: Option<DownstreamInterruption> = None;

    write_stream_prelude(
        stream,
        core,
        context,
        prepared,
        &stream_id,
        &mut downstream_interruption,
    )?;

    match execute_prepared_job_streaming(
        stream,
        core,
        context,
        prepared,
        &stream_id,
        &mut downstream_interruption,
    ) {
        Ok(result) => finish_streaming_success(
            stream,
            core,
            context,
            logger,
            prepared,
            &stream_id,
            downstream_interruption,
            result,
        ),
        Err(err) => {
            let failure_bundle_id =
                capture_gateway_failure_bundle(core, context, prepared, client_id, &err);
            if downstream_interruption.is_none() {
                write_stream_error(
                    stream,
                    &map_openai_error_value(
                        &context.request_id,
                        &err,
                        failure_bundle_id.as_deref(),
                    ),
                )?;
                write_stream_done(stream)?;
            }
            let mut outcome = outcome_from_error(&err);
            if let Some(interruption) = downstream_interruption {
                outcome.interrupted_at = Some(interruption.interrupted_at);
                outcome.interruption_reason = Some(interruption.reason);
                outcome.low_confidence = true;
            }
            Ok(outcome)
        }
    }
}

fn write_stream_prelude(
    stream: &mut TcpStream,
    core: &CoreService,
    context: &RequestContext,
    prepared: &PreparedChatRequest,
    stream_id: &str,
    downstream_interruption: &mut Option<DownstreamInterruption>,
) -> Result<()> {
    if let Err(err) = write_stream_headers(stream, &context.request_id) {
        if is_client_disconnect_error(&err) {
            *downstream_interruption = Some(mark_downstream_interrupted(
                core,
                prepared,
                prepared.conversation_id_hint(),
                stream_disconnect_message(&err),
            ));
            return Ok(());
        }
        return Err(err);
    }

    if let Err(err) = write_stream_role_chunk(stream, stream_id, prepared, context) {
        if is_client_disconnect_error(&err) {
            *downstream_interruption = Some(mark_downstream_interrupted(
                core,
                prepared,
                prepared.conversation_id_hint(),
                stream_disconnect_message(&err),
            ));
            return Ok(());
        }
        return Err(err);
    }

    Ok(())
}

fn execute_prepared_job_streaming(
    stream: &mut TcpStream,
    core: &CoreService,
    context: &RequestContext,
    prepared: &PreparedChatRequest,
    stream_id: &str,
    downstream_interruption: &mut Option<DownstreamInterruption>,
) -> Result<ChatExecutionResult> {
    match &prepared.job {
        PreparedChatJob::Research(job) => core
            .execute_prepared_research(job, |event| {
                write_event_delta_or_record_interruption(
                    stream,
                    core,
                    context,
                    prepared,
                    stream_id,
                    downstream_interruption,
                    event,
                )
            })
            .map(|result| ChatExecutionResult {
                conversation_id: result.conversation_id,
                low_confidence: result.low_confidence,
                recovery_trace: None,
                recovery_packet: None,
            }),
        PreparedChatJob::Ask(job) => core
            .execute_prepared_ask(job, |event| {
                write_event_delta_or_record_interruption(
                    stream,
                    core,
                    context,
                    prepared,
                    stream_id,
                    downstream_interruption,
                    event,
                )
            })
            .map(|result| ChatExecutionResult {
                conversation_id: result.conversation_id,
                low_confidence: result.low_confidence,
                recovery_trace: result.recovery_trace,
                recovery_packet: result.recovery_packet,
            }),
    }
}

fn write_event_delta_or_record_interruption(
    stream: &mut TcpStream,
    core: &CoreService,
    context: &RequestContext,
    prepared: &PreparedChatRequest,
    stream_id: &str,
    downstream_interruption: &mut Option<DownstreamInterruption>,
    event: EventPayload,
) -> Result<()> {
    if downstream_interruption.is_some() {
        return Ok(());
    }
    if let Err(err) = write_stream_delta(
        stream,
        stream_id,
        prepared.model_id,
        &context.request_id,
        event.clone(),
    ) {
        if is_client_disconnect_error(&err) {
            *downstream_interruption = Some(mark_downstream_interrupted(
                core,
                prepared,
                event_conversation_id(&event).as_deref(),
                stream_disconnect_message(&err),
            ));
            return Ok(());
        }
        return Err(err);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish_streaming_success(
    stream: &mut TcpStream,
    core: &CoreService,
    context: &RequestContext,
    logger: &JsonLogger,
    prepared: &PreparedChatRequest,
    stream_id: &str,
    downstream_interruption: Option<DownstreamInterruption>,
    result: ChatExecutionResult,
) -> Result<RequestOutcome> {
    let recovered_at = recovered_at_for_trace(result.recovery_trace.as_ref());
    let recovery_reason = result
        .recovery_trace
        .as_ref()
        .filter(|trace| trace.recovery_attempted)
        .and_then(recovery_reason_for_trace);

    if let Some(interruption) = downstream_interruption {
        return recover_downstream_interruption(
            core,
            Some(logger),
            context,
            prepared,
            interruption,
            DownstreamRecovery {
                conversation_id: Some(result.conversation_id),
                recovery_trace: result.recovery_trace,
                recovery_packet: result.recovery_packet,
            },
        );
    }

    if let Err(err) = write_stream_finish_chunk(stream, stream_id, prepared, context) {
        if is_client_disconnect_error(&err) {
            let interruption = mark_downstream_interrupted(
                core,
                prepared,
                Some(&result.conversation_id),
                stream_disconnect_message(&err),
            );
            return recover_downstream_interruption(
                core,
                None,
                context,
                prepared,
                interruption,
                DownstreamRecovery {
                    conversation_id: Some(result.conversation_id),
                    recovery_trace: result.recovery_trace,
                    recovery_packet: result.recovery_packet,
                },
            );
        }
        return Err(err);
    }
    if let Err(err) = write_stream_done(stream) {
        if is_client_disconnect_error(&err) {
            let interruption = mark_downstream_interrupted(
                core,
                prepared,
                Some(&result.conversation_id),
                stream_disconnect_message(&err),
            );
            return recover_downstream_interruption(
                core,
                None,
                context,
                prepared,
                interruption,
                DownstreamRecovery {
                    conversation_id: Some(result.conversation_id),
                    recovery_trace: result.recovery_trace,
                    recovery_packet: result.recovery_packet,
                },
            );
        }
        return Err(err);
    }
    logger.info(
        "gateway streamed chat completion",
        serde_json::json!({
            "request_id": context.request_id,
            "model": prepared.model_id,
            "job_id": prepared.job_id(),
        }),
    )?;
    Ok(RequestOutcome {
        kind: if result.low_confidence {
            RequestOutcomeKind::Recovered
        } else {
            RequestOutcomeKind::Completed
        },
        low_confidence: result.low_confidence,
        conversation_id: Some(result.conversation_id),
        interrupted_at: None,
        interruption_reason: None,
        recovered_at,
        recovery_reason,
        failed_at: None,
        failure_reason: None,
        recovery_trace: result.recovery_trace,
        recovery_packet: result.recovery_packet,
    })
}

fn execute_non_streaming_chat_completion(
    stream: &mut TcpStream,
    core: &CoreService,
    context: &RequestContext,
    logger: &JsonLogger,
    prepared: &PreparedChatRequest,
) -> Result<RequestOutcome> {
    let result = execute_prepared_job_non_streaming(core, context, prepared)?;
    log_response(logger, &context.request_id, result.response.status_code)?;
    if let Err(err) = write_http_response(stream, &context.request_id, result.response) {
        if is_client_disconnect_error(&err) {
            let interruption = mark_downstream_interrupted(
                core,
                prepared,
                result.conversation_id.as_deref(),
                final_response_disconnect_message(&err),
            );
            return recover_downstream_interruption(
                core,
                None,
                context,
                prepared,
                interruption,
                DownstreamRecovery {
                    conversation_id: result.conversation_id,
                    recovery_trace: result.recovery_trace,
                    recovery_packet: result.recovery_packet,
                },
            );
        }
        return Err(err);
    }
    let recovered_at = recovered_at_for_trace(result.recovery_trace.as_ref());
    let recovery_reason = result
        .recovery_trace
        .as_ref()
        .filter(|trace| trace.recovery_attempted)
        .and_then(recovery_reason_for_trace);
    Ok(RequestOutcome {
        kind: if result.low_confidence {
            RequestOutcomeKind::Recovered
        } else {
            RequestOutcomeKind::Completed
        },
        low_confidence: result.low_confidence,
        conversation_id: result.conversation_id,
        interrupted_at: None,
        interruption_reason: None,
        recovered_at,
        recovery_reason,
        failed_at: None,
        failure_reason: None,
        recovery_trace: result.recovery_trace,
        recovery_packet: result.recovery_packet,
    })
}

fn execute_prepared_job_non_streaming(
    core: &CoreService,
    context: &RequestContext,
    prepared: &PreparedChatRequest,
) -> Result<NonStreamingExecution> {
    match &prepared.job {
        PreparedChatJob::Research(job) => {
            let result = core.execute_prepared_research(job, |_event| Ok(()))?;
            let conversation = core.find_conversation(&result.conversation_id)?;
            Ok(NonStreamingExecution {
                response: build_chat_completion_response(
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
                conversation_id: Some(result.conversation_id),
                low_confidence: result.low_confidence,
                recovery_trace: None,
                recovery_packet: None,
            })
        }
        PreparedChatJob::Ask(job) => {
            let result = core.execute_prepared_ask(job, |_event| Ok(()))?;
            let conversation = core.find_conversation(&result.conversation_id)?;
            Ok(NonStreamingExecution {
                response: build_chat_completion_response(
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
                conversation_id: Some(result.conversation_id),
                low_confidence: result.low_confidence,
                recovery_trace: result.recovery_trace,
                recovery_packet: result.recovery_packet,
            })
        }
    }
}

fn recovered_at_for_trace(recovery_trace: Option<&RecoveryTrace>) -> Option<chrono::DateTime<Utc>> {
    recovery_trace
        .filter(|trace| trace.recovery_attempted)
        .map(|_| Utc::now())
}
