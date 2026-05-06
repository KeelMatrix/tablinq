use crate::http::{HttpRequest, HttpResponse, RequestContext};
use crate::json::{artifact_object, json_response, reason_phrase_for_status};
use crate::routes::{
    cancel_job_id_from_path, cancel_request_id_from_path, job_id_from_path,
    parse_request_list_query, request_evidence_request_id_from_path, request_id_from_path,
};
use crate::scheduler::{CancelDisposition, GatewayScheduler, SchedulerSnapshot};
use anyhow::Result;
use gptd::CoreService;
use serde_json::{Value, json};
use shared_protocol::CancelJobCommand;

pub(crate) fn handle_requests_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let query = match parse_request_list_query(&context.request.path, &context.request_id) {
        Ok(query) => query,
        Err(response) => return Ok(response),
    };
    let page = core.list_request_page(query.status, query.limit, query.offset)?;
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.request_list",
            "request_id": context.request_id,
            "data": page.requests,
            "pagination": {
                "limit": page.limit,
                "offset": page.offset,
                "next_offset": page.next_offset,
                "has_more": page.next_offset.is_some(),
            },
            "filter": {
                "status": page.status,
            },
        }),
    ))
}

pub(crate) fn handle_request_summary_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(target_request_id) = request_id_from_path(&context.request.path) else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };

    let summary = match core.request_summary(target_request_id) {
        Ok(summary) => summary,
        Err(_) => {
            return Ok(json_response(
                404,
                json!({
                    "request_id": context.request_id,
                    "error": "not_found",
                }),
            ));
        }
    };

    Ok(json_response(
        200,
        json!({
            "object": "tablinq.request_summary",
            "request_id": context.request_id,
            "data": summary,
        }),
    ))
}

pub(crate) fn handle_request_evidence_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(target_request_id) = request_evidence_request_id_from_path(&context.request.path)
    else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };

    let artifact = match core.latest_request_evidence(target_request_id) {
        Ok(artifact) => artifact,
        Err(_) => {
            return Ok(json_response(
                404,
                json!({
                    "request_id": context.request_id,
                    "error": "not_found",
                }),
            ));
        }
    };

    Ok(json_response(
        200,
        json!({
            "object": "tablinq.request_evidence",
            "request_id": context.request_id,
            "data": artifact_object(core, artifact)?,
        }),
    ))
}

pub(crate) fn handle_cancel_request(
    core: &CoreService,
    scheduler: &GatewayScheduler,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(request_id) = cancel_request_id_from_path(&context.request.path) else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };

    let Some((job_id, disposition)) = scheduler.cancel(request_id)? else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };

    let ack = core.cancel_job(shared_protocol::CancelJobCommand {
        job_id: job_id.clone(),
    })?;
    Ok(json_response(
        200,
        json!({
            "request_id": context.request_id,
            "target_request_id": request_id,
            "job_id": job_id,
            "accepted": ack.accepted,
            "message": ack.message,
            "state": match disposition {
                CancelDisposition::Queued => "queued",
                CancelDisposition::Running => "running",
            },
        }),
    ))
}

pub(crate) fn handle_jobs_request(core: &CoreService, request_id: &str) -> Result<HttpResponse> {
    let jobs = core.list_jobs()?;
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.job_list",
            "request_id": request_id,
            "data": jobs.jobs,
        }),
    ))
}

pub(crate) fn handle_job_detail_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(job_id) = job_id_from_path(&context.request.path) else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };

    let detail = match core.job_detail(job_id) {
        Ok(detail) => detail,
        Err(_) => {
            return Ok(json_response(
                404,
                json!({
                    "request_id": context.request_id,
                    "error": "not_found",
                }),
            ));
        }
    };

    Ok(json_response(
        200,
        json!({
            "object": "tablinq.job",
            "request_id": context.request_id,
            "data": detail,
        }),
    ))
}

pub(crate) fn handle_job_cancel_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(job_id) = cancel_job_id_from_path(&context.request.path) else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };

    let ack = match core.cancel_job(CancelJobCommand {
        job_id: job_id.to_owned(),
    }) {
        Ok(ack) => ack,
        Err(_) => {
            return Ok(json_response(
                404,
                json!({
                    "request_id": context.request_id,
                    "error": "not_found",
                }),
            ));
        }
    };

    Ok(json_response(
        200,
        json!({
            "object": "tablinq.ack",
            "request_id": context.request_id,
            "job_id": job_id,
            "accepted": ack.accepted,
            "message": ack.message,
        }),
    ))
}

pub(crate) fn client_id_for_request(request: &HttpRequest, remote_addr: &str) -> String {
    request
        .headers
        .get("x-client-id")
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            remote_addr
                .rsplit_once(':')
                .map(|(host, _)| host.to_owned())
                .unwrap_or_else(|| remote_addr.to_owned())
        })
}

pub(crate) fn queue_error_response(
    request_id: &str,
    scheduler: &SchedulerSnapshot,
    err: &anyhow::Error,
) -> HttpResponse {
    let message = format!("{err:#}");
    let (status, error_type, code) = if message.contains("queue timeout") {
        (429, "overloaded_error", "queue_timeout")
    } else if message.contains("per-client queue limit") {
        (429, "overloaded_error", "client_limit_exceeded")
    } else if message.contains("cancelled") {
        (409, "invalid_request_error", "cancelled")
    } else {
        (429, "overloaded_error", "queue_full")
    };
    let body = json!({
        "error": {
            "message": message,
            "type": error_type,
            "param": Value::Null,
            "code": code,
        },
        "request_id": request_id,
        "retry_after_ms": scheduler.queue_timeout_ms,
        "scheduler": scheduler,
    });
    HttpResponse {
        status_code: status,
        reason_phrase: reason_phrase_for_status(status),
        headers: vec![
            ("Content-Type".to_owned(), "application/json".to_owned()),
            ("Retry-After".to_owned(), "1".to_owned()),
        ],
        body: serde_json::to_vec(&body).unwrap_or_default(),
    }
}

pub(crate) fn cancellation_response(request_id: &str, job_id: &str, message: &str) -> HttpResponse {
    json_response(
        409,
        json!({
            "error": {
                "message": message,
                "type": "invalid_request_error",
                "param": Value::Null,
                "code": "cancelled",
            },
            "request_id": request_id,
            "job_id": job_id,
            "cancelled": true,
            "job_status": "cancelled",
        }),
    )
}
