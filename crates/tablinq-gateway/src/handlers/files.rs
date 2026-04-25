use crate::http::{HttpResponse, RequestContext};
use crate::json::{json_response, staged_file_object};
use crate::routes::file_id_from_path;
use crate::uploads::UploadStore;
use anyhow::Result;
use serde_json::json;

pub(crate) fn handle_files_list_request(
    upload_store: &UploadStore,
    request_id: &str,
) -> Result<HttpResponse> {
    let files = upload_store
        .list()?
        .into_iter()
        .map(staged_file_object)
        .collect::<Vec<_>>();
    Ok(json_response(
        200,
        json!({
            "object": "list",
            "request_id": request_id,
            "data": files,
        }),
    ))
}

pub(crate) fn handle_file_upload_request(
    upload_store: &UploadStore,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(file_name) = context
        .request
        .headers
        .get("x-file-name")
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(json_response(
            400,
            json!({
                "request_id": context.request_id,
                "error": "x-file-name header is required",
            }),
        ));
    };
    if context.request.body.len() > crate::uploads::MAX_STAGED_UPLOAD_BYTES {
        return Ok(json_response(
            413,
            json!({
                "request_id": context.request_id,
                "error": "file exceeds 20 MiB limit",
            }),
        ));
    }
    let metadata = upload_store.stage_upload(
        file_name,
        context
            .request
            .headers
            .get("content-type")
            .map(String::as_str),
        &context.request.body,
    )?;
    Ok(json_response(
        201,
        json!({
            "request_id": context.request_id,
            "data": staged_file_object(metadata),
        }),
    ))
}

pub(crate) fn handle_file_detail_request(
    upload_store: &UploadStore,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(file_id) = file_id_from_path(&context.request.path) else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };
    let Some(metadata) = upload_store.get(file_id)? else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };
    Ok(json_response(
        200,
        json!({
            "request_id": context.request_id,
            "data": staged_file_object(metadata),
        }),
    ))
}
