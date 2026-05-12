use super::interruption::{
    DownstreamRecovery, final_response_disconnect_message, mark_downstream_interrupted,
    recover_downstream_interruption, stream_disconnect_message,
};
use super::job_execution::{ChatJobKind, execute_prepared_chat_job};
use super::preparation::PreparedChatRequest;
use super::response::{build_chat_completion_response, recovery_reason_for_trace};
use super::sse_writer::{
    write_stream_content_chunk, write_stream_done, write_stream_error, write_stream_finish_chunk,
    write_stream_headers,
};
use super::{RequestOutcome, RequestOutcomeKind, outcome_from_error};
use crate::browser::ensure_browser_session;
use crate::errors::{
    capture_gateway_failure_bundle, is_client_disconnect_error, map_openai_error_value,
};
use crate::http::{HttpResponse, RequestContext, log_response, write_http_response};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use gptd::CoreService;
use gptd::logging::JsonLogger;
use shared_protocol::{RecoveryPacket, RecoveryTrace};
use std::net::TcpStream;

struct NonStreamingExecution {
    response: HttpResponse,
    conversation_id: Option<String>,
    session_id: Option<String>,
    content: String,
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
    if prepared.request.stream {
        execute_streaming_chat_completion(stream, core, context, logger, prepared, client_id)
    } else {
        ensure_browser_session(core)?;
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
    match ensure_browser_session(core)
        .and_then(|_| execute_prepared_job_non_streaming(core, context, prepared))
    {
        Ok(result) => {
            finish_streaming_success(stream, core, context, logger, prepared, &stream_id, result)
        }
        Err(err) => {
            let failure_bundle_id =
                capture_gateway_failure_bundle(core, context, prepared, client_id, &err);
            if !peer_closed_before_final_response(stream)? {
                write_stream_headers(stream, &context.request_id)?;
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
            Ok(outcome_from_error(&err))
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn finish_streaming_success(
    stream: &mut TcpStream,
    core: &CoreService,
    context: &RequestContext,
    logger: &JsonLogger,
    prepared: &PreparedChatRequest,
    stream_id: &str,
    result: NonStreamingExecution,
) -> Result<RequestOutcome> {
    let recovered_at = recovered_at_for_trace(result.recovery_trace.as_ref());
    let recovery_reason = result
        .recovery_trace
        .as_ref()
        .filter(|trace| trace.recovery_attempted)
        .and_then(recovery_reason_for_trace);

    if peer_closed_before_final_response(stream)? {
        let err = anyhow!("client closed before final streamed response");
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

    if let Err(err) = write_stream_headers(stream, &context.request_id) {
        if is_client_disconnect_error(&err) {
            let interruption = mark_downstream_interrupted(
                core,
                prepared,
                result.conversation_id.as_deref(),
                stream_disconnect_message(&err),
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
    if let Err(err) = write_stream_content_chunk(
        stream,
        stream_id,
        prepared,
        context,
        result.conversation_id.as_deref(),
        result.session_id.as_deref(),
        &result.content,
    ) {
        if is_client_disconnect_error(&err) {
            let interruption = mark_downstream_interrupted(
                core,
                prepared,
                result.conversation_id.as_deref(),
                stream_disconnect_message(&err),
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
    if let Err(err) = write_stream_finish_chunk(
        stream,
        stream_id,
        prepared,
        context,
        result.conversation_id.as_deref(),
        result.session_id.as_deref(),
    ) {
        if is_client_disconnect_error(&err) {
            let interruption = mark_downstream_interrupted(
                core,
                prepared,
                result.conversation_id.as_deref(),
                stream_disconnect_message(&err),
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
    if let Err(err) = write_stream_done(stream) {
        if is_client_disconnect_error(&err) {
            let interruption = mark_downstream_interrupted(
                core,
                prepared,
                result.conversation_id.as_deref(),
                stream_disconnect_message(&err),
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
    let _ = logger.info(
        "gateway completed final-only streamed chat completion",
        serde_json::json!({
            "request_id": context.request_id,
            "model": prepared.model_id,
            "job_id": prepared.job_id(),
        }),
    );
    Ok(RequestOutcome {
        kind: successful_outcome_kind(result.recovery_trace.as_ref()),
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

fn execute_non_streaming_chat_completion(
    stream: &mut TcpStream,
    core: &CoreService,
    context: &RequestContext,
    logger: &JsonLogger,
    prepared: &PreparedChatRequest,
) -> Result<RequestOutcome> {
    let result = execute_prepared_job_non_streaming(core, context, prepared)?;
    if peer_closed_before_final_response(stream)? {
        let err = anyhow!("client closed before final response");
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
    let _ = log_response(logger, &context.request_id, result.response.status_code);
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
        kind: successful_outcome_kind(result.recovery_trace.as_ref()),
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
    let result = execute_prepared_chat_job(core, prepared, |_event| Ok(()))?;
    let conversation = core.find_conversation(&result.conversation_id)?;
    let session_id = conversation.as_ref().map(|item| item.session_id.clone());
    let recovery_trace = match result.kind {
        ChatJobKind::Ask => result.recovery_trace.as_ref(),
        ChatJobKind::Research => None,
    };
    let response = build_chat_completion_response(
        &context.request_id,
        prepared.job_id(),
        prepared.model_id,
        prepared.request_role,
        prepared.parent_request_id.as_deref(),
        Some(&result.conversation_id),
        session_id.as_deref(),
        &result.output_text,
        "stop",
        recovery_trace,
    );

    Ok(NonStreamingExecution {
        response,
        conversation_id: Some(result.conversation_id),
        session_id,
        content: result.output_text,
        low_confidence: result.low_confidence,
        recovery_trace: result.recovery_trace,
        recovery_packet: result.recovery_packet,
    })
}

fn peer_closed_before_final_response(stream: &TcpStream) -> Result<bool> {
    stream
        .set_nonblocking(true)
        .context("set final response peer probe nonblocking")?;
    let mut byte = [0u8; 1];
    let probe_result = match stream.peek(&mut byte) {
        Ok(0) => Ok(true),
        Ok(_) => Ok(false),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(false),
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::UnexpectedEof
            ) =>
        {
            Ok(true)
        }
        Err(err) => Err(err).context("probe final response peer"),
    };
    let restore_result = stream
        .set_nonblocking(false)
        .context("restore final response peer probe blocking");
    restore_result?;
    probe_result
}

fn recovered_at_for_trace(recovery_trace: Option<&RecoveryTrace>) -> Option<chrono::DateTime<Utc>> {
    recovery_trace
        .filter(|trace| trace.recovery_attempted)
        .map(|_| Utc::now())
}

fn successful_outcome_kind(recovery_trace: Option<&RecoveryTrace>) -> RequestOutcomeKind {
    if recovery_trace
        .map(|trace| trace.recovery_attempted)
        .unwrap_or(false)
    {
        RequestOutcomeKind::Recovered
    } else {
        RequestOutcomeKind::Completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_protocol::{ErrorCode, RecoveryStrategy};

    #[test]
    fn successful_outcome_does_not_treat_low_confidence_as_recovered() {
        assert_eq!(successful_outcome_kind(None), RequestOutcomeKind::Completed);
    }

    #[test]
    fn successful_outcome_is_recovered_only_after_recovery_attempt() {
        let trace = RecoveryTrace {
            recovery_available: true,
            recovery_attempted: true,
            recovery_strategy: Some(RecoveryStrategy::ReloadNewConversationReplayWithContext),
            failure_code: Some(ErrorCode::ProviderGenerationFailedNonretryable),
            failure_bundle_id: Some("bundle-1".to_owned()),
            prior_request_id: Some("job-1:provider-attempt-0".to_owned()),
            replacement_request_id: Some("job-1:provider-attempt-1".to_owned()),
            prior_conversation_id: Some("conv-old".to_owned()),
            replacement_conversation_id: Some("conv-new".to_owned()),
            recovery_attempt_count: 1,
        };

        assert_eq!(
            successful_outcome_kind(Some(&trace)),
            RequestOutcomeKind::Recovered
        );
    }
}
