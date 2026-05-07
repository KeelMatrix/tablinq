use crate::http::{HttpResponse, RequestContext};
use crate::json::{api_error_response, json_response, staged_file_object};
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
        return Ok(api_error_response(
            400,
            &context.request_id,
            "invalid_request_error",
            "missing_file_name_header",
            "x-file-name header is required",
        ));
    };
    if context.request.body.len() > crate::uploads::MAX_STAGED_UPLOAD_BYTES {
        return Ok(api_error_response(
            413,
            &context.request_id,
            "invalid_request_error",
            "payload_too_large",
            "file exceeds 20 MiB limit",
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
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            "staged file was not found",
        ));
    };
    let Some(metadata) = upload_store.get(file_id)? else {
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            format!("staged file {file_id} was not found"),
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
