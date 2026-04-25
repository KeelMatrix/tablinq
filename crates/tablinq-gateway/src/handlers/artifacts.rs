use crate::http::{HttpResponse, RequestContext};
use crate::json::{artifact_object, json_response, reason_phrase_for_status};
use crate::routes::artifact_id_from_path;
use anyhow::{Context, Result};
use gptd::CoreService;
use serde_json::json;
use std::fs;

pub(crate) fn handle_artifacts_list_request(
    core: &CoreService,
    request_id: &str,
) -> Result<HttpResponse> {
    let artifacts = core
        .list_artifacts()?
        .artifacts
        .into_iter()
        .map(|artifact| artifact_object(core, artifact))
        .collect::<Result<Vec<_>>>()?;
    Ok(json_response(
        200,
        json!({
            "object": "list",
            "request_id": request_id,
            "data": artifacts,
        }),
    ))
}

pub(crate) fn handle_artifact_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some((artifact_id, wants_content)) = artifact_id_from_path(&context.request.path) else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };
    let artifact = core
        .list_artifacts()?
        .artifacts
        .into_iter()
        .find(|item| item.artifact_id == artifact_id);
    let Some(artifact) = artifact else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
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
