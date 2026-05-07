use crate::chat::{
    RequestOutcome, RequestOutcomeKind, build_request_summary, execute_chat_completions_request,
    outcome_from_error, prepare_chat_completions_request,
};
use crate::errors::{
    capture_gateway_failure_artifacts, map_openai_error, maybe_capture_request_evidence,
};
use crate::handlers::requests::{
    cancellation_response, client_id_for_request, queue_error_response,
};
use crate::http::{RequestContext, log_response, write_http_response};
use crate::scheduler::{GatewayScheduler, SchedulerSnapshot};
use crate::uploads::UploadStore;
use anyhow::Result;
use chrono::Utc;
use gptd::CoreService;
use gptd::logging::JsonLogger;
use std::net::TcpStream;
use std::time::Instant;

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_chat_completions_request(
    stream: &mut TcpStream,
    core: &CoreService,
    upload_store: &UploadStore,
    scheduler: &GatewayScheduler,
    scheduler_status: SchedulerSnapshot,
    context: &RequestContext,
    logger: &JsonLogger,
    remote_addr: &str,
) -> Result<()> {
    let prepared = match prepare_chat_completions_request(core, upload_store, context) {
        Ok(prepared) => prepared,
        Err(err) => {
            let response = map_openai_error(&context.request_id, &err, None);
            log_response(logger, &context.request_id, response.status_code)?;
            return write_http_response(stream, &context.request_id, response);
        }
    };
    let _ = upload_store.mark_linked_usage(
        &prepared.file_ids,
        &context.request_id,
        prepared.conversation_id_hint(),
    );
    let request_started_at = Utc::now();
    let client_id = client_id_for_request(&context.request, remote_addr);
    let queue_wait_started_at = Instant::now();
    let _lease = match scheduler.acquire(&context.request_id, prepared.job_id(), &client_id) {
        Ok(lease) => lease,
        Err(err) => {
            let queue_wait_ms = queue_wait_started_at.elapsed().as_millis() as u64;
            if format!("{err:#}").contains("cancelled") {
                let _ = core.mark_job_cancelled(prepared.job_id(), &format!("{err:#}"));
            } else {
                let _ = core.fail_job(prepared.job_id(), &format!("{err:#}"));
            }
            let response = queue_error_response(
                &context.request_id,
                prepared.job_id(),
                Some(crate::openai::request_role_name(prepared.request_role)),
                prepared.parent_request_id.as_deref(),
                &scheduler.snapshot().unwrap_or(scheduler_status),
                &err,
            );
            let _ = core.record_request_summary(&build_request_summary(
                context,
                &prepared,
                request_started_at,
                Utc::now(),
                queue_wait_ms,
                outcome_from_error(&err),
            ));
            log_response(logger, &context.request_id, response.status_code)?;
            return write_http_response(stream, &context.request_id, response);
        }
    };
    let queue_wait_ms = queue_wait_started_at.elapsed().as_millis() as u64;
    if core
        .job_cancel_requested(prepared.job_id())
        .unwrap_or(false)
    {
        let _ = core.mark_job_cancelled(prepared.job_id(), "request cancelled before execution");
        let response = cancellation_response(
            &context.request_id,
            prepared.job_id(),
            "request cancelled before execution",
        );
        let _ = core.record_request_summary(&build_request_summary(
            context,
            &prepared,
            request_started_at,
            Utc::now(),
            queue_wait_ms,
            RequestOutcome {
                kind: RequestOutcomeKind::Cancelled,
                low_confidence: false,
                conversation_id: prepared.conversation_id_hint().map(str::to_owned),
                interrupted_at: None,
                interruption_reason: None,
                recovered_at: None,
                recovery_reason: None,
                failed_at: None,
                failure_reason: None,
                recovery_trace: None,
                recovery_packet: None,
            },
        ));
        log_response(logger, &context.request_id, response.status_code)?;
        return write_http_response(stream, &context.request_id, response);
    }
    match execute_chat_completions_request(stream, core, context, logger, &prepared, &client_id) {
        Ok(outcome) => {
            let _ = upload_store.mark_linked_usage(
                &prepared.file_ids,
                &context.request_id,
                outcome.conversation_id.as_deref(),
            );
            let should_capture_evidence = outcome.low_confidence;
            let _ = core.record_request_summary(&build_request_summary(
                context,
                &prepared,
                request_started_at,
                Utc::now(),
                queue_wait_ms,
                outcome,
            ));
            if should_capture_evidence {
                let _ = maybe_capture_request_evidence(core, &context.request_id, None, true);
            }
        }
        Err(err) => {
            let _ = core.record_request_summary(&build_request_summary(
                context,
                &prepared,
                request_started_at,
                Utc::now(),
                queue_wait_ms,
                outcome_from_error(&err),
            ));
            let failure_bundle_id =
                capture_gateway_failure_artifacts(core, context, &prepared, &client_id, &err);
            let response =
                map_openai_error(&context.request_id, &err, failure_bundle_id.as_deref());
            log_response(logger, &context.request_id, response.status_code)?;
            return write_http_response(stream, &context.request_id, response);
        }
    }
    Ok(())
}
