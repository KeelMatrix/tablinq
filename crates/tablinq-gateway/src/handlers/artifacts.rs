use crate::http::{HttpResponse, RequestContext};
use crate::json::{
    api_error_response, artifact_object, compact_artifact_object, json_response, pagination_object,
    reason_phrase_for_status, response_meta_object,
};
use crate::routes::{artifact_id_from_path, paginate_items, parse_state_list_query};
use anyhow::{Context, Result};
use gptd::CoreService;
use serde_json::json;
use std::fs;

pub(crate) fn handle_artifacts_list_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let query = match parse_state_list_query(&context.request.path, &context.request_id, &[]) {
        Ok(query) => query,
        Err(response) => return Ok(response),
    };
    let compact = query.wants_summary();
    let page = paginate_items(core.list_artifacts()?.artifacts, query.pagination);
    let pagination = pagination_object(&page);
    let artifacts = page
        .items
        .into_iter()
        .map(|artifact| {
            if compact {
                Ok(compact_artifact_object(artifact))
            } else {
                artifact_object(core, artifact)
            }
        })
        .collect::<Result<Vec<_>>>()?;
    let truncated_fields = if compact {
        vec!["linked_job_ids", "linked_request_ids"]
    } else {
        vec![]
    };
    Ok(json_response(
        200,
        json!({
            "object": "list",
            "request_id": context.request_id,
            "data": artifacts,
            "pagination": pagination,
            "meta": response_meta_object(query.view, query.compact, truncated_fields),
        }),
    ))
}

pub(crate) fn handle_artifact_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some((artifact_id, wants_content)) = artifact_id_from_path(&context.request.path) else {
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            "artifact was not found",
        ));
    };
    let artifact = core
        .list_artifacts()?
        .artifacts
        .into_iter()
        .find(|item| item.artifact_id == artifact_id);
    let Some(artifact) = artifact else {
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            format!("artifact {artifact_id} was not found"),
        ));
    };

    if wants_content {
        let body = fs::read(&artifact.storage_path)
            .with_context(|| format!("read {}", artifact.storage_path))?;
        return Ok(HttpResponse {
            status_code: 200,
            reason_phrase: reason_phrase_for_status(200),
            headers: vec![
                ("Content-Type".to_owned(), artifact.content_type.clone()),
                (
                    "Content-Disposition".to_owned(),
                    format!("attachment; filename=\"{}\"", artifact.file_name),
                ),
            ],
            body,
        });
    }

    Ok(json_response(
        200,
        json!({
            "request_id": context.request_id,
            "data": artifact_object(core, artifact)?,
        }),
    ))
}
