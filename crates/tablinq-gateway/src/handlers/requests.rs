use crate::http::{HttpRequest, HttpResponse, RequestContext};
use crate::json::{
    api_error_response, artifact_object, json_response, reason_phrase_for_status,
    response_meta_object,
};
use crate::routes::{
    ResponseView, cancel_job_id_from_path, cancel_request_id_from_path, includes_value,
    job_id_from_path, paginate_items, parse_request_detail_query, parse_request_evidence_query,
    parse_request_list_query, parse_state_list_query, request_evidence_request_id_from_path,
    request_id_from_path,
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
    let limit = query
        .pagination
        .limit
        .unwrap_or(crate::routes::REQUEST_SUMMARY_LIMIT);
    let page = core.list_request_page(query.status, limit, query.pagination.offset)?;
    let returned = page.requests.len();
    let data = page.requests;
    let page_limit = page.limit;
    let page_offset = page.offset;
    let next_offset = page.next_offset;
    let status = page.status;
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.request_list",
            "request_id": context.request_id,
            "data": data,
            "pagination": {
                "limit": page_limit,
                "offset": page_offset,
                "returned": returned,
                "next_offset": next_offset,
                "has_more": next_offset.is_some(),
                "total": Value::Null,
            },
            "meta": response_meta_object(query.view, query.compact, vec![]),
            "filter": {
                "status": status,
            },
        }),
    ))
}

pub(crate) fn handle_request_summary_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(target_request_id) = request_id_from_path(&context.request.path) else {
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            "request was not found",
        ));
    };
    let query = match parse_request_detail_query(&context.request.path, &context.request_id) {
        Ok(query) => query,
        Err(response) => return Ok(response),
    };

    let summary = match core.request_summary(target_request_id) {
        Ok(summary) => summary,
        Err(_) => {
            return Ok(api_error_response(
                404,
                &context.request_id,
                "not_found_error",
                "not_found",
                format!("request {target_request_id} was not found"),
            ));
        }
    };
    let raw_summary = serde_json::to_value(&summary).unwrap_or_else(|_| json!({}));
    let mut data = if query.view.is_summary() || query.compact {
        json!({
            "request_id": summary.request_id,
            "job_id": summary.job_id,
            "model_id": summary.model_id,
            "request_role": summary.request_role,
            "parent_request_id": summary.parent_request_id,
            "execution_kind": summary.execution_kind,
            "status": summary.status,
            "cancelled": summary.cancelled,
            "start_time": summary.start_time,
            "end_time": summary.end_time,
            "duration_ms": summary.duration_ms,
            "queue_wait_ms": summary.queue_wait_ms,
        })
    } else {
        raw_summary.clone()
    };
    let mut truncated_fields = Vec::new();
    if query.view.is_summary() || query.compact {
        truncated_fields.extend(["recovery_trace", "recovery_packet", "usage", "warnings"]);
    }
    if includes_value(&query.includes, "evidence")
        && let Some(object) = data.as_object_mut()
    {
        let evidence = match core.latest_request_evidence(target_request_id) {
            Ok(artifact) => artifact_object(core, artifact)?,
            Err(_) => Value::Null,
        };
        object.insert("evidence".to_owned(), evidence);
    }
    if includes_value(&query.includes, "debug")
        && let Some(object) = data.as_object_mut()
    {
        object.insert(
            "debug".to_owned(),
            json!({
                "recovery_trace": raw_summary["recovery_trace"].clone(),
                "recovery_packet": raw_summary["recovery_packet"].clone(),
                "warnings": raw_summary["warnings"].clone(),
            }),
        );
    }
    if includes_value(&query.includes, "raw")
        && let Some(object) = data.as_object_mut()
    {
        object.insert("raw".to_owned(), raw_summary);
    }

    Ok(json_response(
        200,
        json!({
            "object": "tablinq.request_summary",
            "request_id": context.request_id,
            "data": data,
            "meta": {
                "view": query.view.as_str(),
                "compact": query.compact,
                "include": query.includes,
                "truncated": !truncated_fields.is_empty(),
                "truncated_fields": truncated_fields,
            },
        }),
    ))
}

pub(crate) fn handle_request_evidence_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(target_request_id) = request_evidence_request_id_from_path(&context.request.path)
    else {
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            "request evidence was not found",
        ));
    };
    let pagination = match parse_request_evidence_query(&context.request.path, &context.request_id)
    {
        Ok(query) => query,
        Err(response) => return Ok(response),
    };

    let artifact = match core.latest_request_evidence(target_request_id) {
        Ok(artifact) => artifact,
        Err(_) => {
            return Ok(api_error_response(
                404,
                &context.request_id,
                "not_found_error",
                "not_found",
                format!("request evidence for {target_request_id} was not found"),
            ));
        }
    };
    let page = paginate_items(vec![artifact], pagination);
    let limit = page.limit;
    let offset = page.offset;
    let returned = page.returned;
    let next_offset = page.next_offset;
    let has_more = page.has_more;
    let total = page.total;
    let data = page
        .items
        .into_iter()
        .next()
        .map(|artifact| artifact_object(core, artifact))
        .transpose()?;

    Ok(json_response(
        200,
        json!({
            "object": "tablinq.request_evidence",
            "request_id": context.request_id,
            "data": data,
            "pagination": {
                "limit": limit,
                "offset": offset,
                "returned": returned,
                "next_offset": next_offset,
                "has_more": has_more,
                "total": total,
            },
            "meta": response_meta_object(ResponseView::Detail, false, vec![]),
        }),
    ))
}

pub(crate) fn handle_cancel_request(
    core: &CoreService,
    scheduler: &GatewayScheduler,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(request_id) = cancel_request_id_from_path(&context.request.path) else {
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            "request was not found",
        ));
    };

    let Some((job_id, disposition)) = scheduler.cancel(request_id)? else {
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            format!("request {request_id} was not found"),
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

pub(crate) fn handle_jobs_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let query = match parse_state_list_query(&context.request.path, &context.request_id, &[]) {
        Ok(query) => query,
        Err(response) => return Ok(response),
    };
    let jobs = core.list_jobs()?;
    let page = paginate_items(jobs.jobs, query.pagination);
    let pagination = crate::json::pagination_object(&page);
    let data = page.items;
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.job_list",
            "request_id": context.request_id,
            "data": data,
            "pagination": pagination,
            "meta": response_meta_object(query.view, query.compact, vec![]),
        }),
    ))
}

pub(crate) fn handle_job_detail_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(job_id) = job_id_from_path(&context.request.path) else {
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            "job was not found",
        ));
    };

    let detail = match core.job_detail(job_id) {
        Ok(detail) => detail,
        Err(_) => {
            return Ok(api_error_response(
                404,
                &context.request_id,
                "not_found_error",
                "not_found",
                format!("job {job_id} was not found"),
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
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            "job was not found",
        ));
    };

    let ack = match core.cancel_job(CancelJobCommand {
        job_id: job_id.to_owned(),
    }) {
        Ok(ack) => ack,
        Err(_) => {
            return Ok(api_error_response(
                404,
                &context.request_id,
                "not_found_error",
                "not_found",
                format!("job {job_id} was not found"),
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
    job_id: &str,
    request_role: Option<&str>,
    parent_request_id: Option<&str>,
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
        "job_id": job_id,
        "tablinq_request_role": request_role,
        "tablinq_parent_request_id": parent_request_id,
        "tablinq_queue_state": "queued",
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
