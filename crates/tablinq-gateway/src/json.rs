use crate::http::{HttpResponse, json_response as http_json_response};
use crate::routes::{V1_ARTIFACTS_PATH, V1_CONVERSATIONS_SELECT_PATH};
use crate::uploads::StagedFileMetadata;
use anyhow::Result;
use gptd::CoreService;
use serde_json::{Value, json};
use shared_protocol::{
    ArtifactSummary, ConversationDetail, ConversationSelection, ConversationSnapshot,
    ConversationSummary,
};

pub(crate) fn json_response(status_code: u16, body: Value) -> HttpResponse {
    http_json_response(status_code, body)
}

pub(crate) fn reason_phrase_for_status(status_code: u16) -> &'static str {
    crate::http::reason_phrase_for_status(status_code)
}

pub(crate) fn conversation_summary_object(summary: ConversationSummary) -> Value {
    let conversation_id = summary.conversation_id.clone();
    json!({
        "conversation_id": conversation_id.clone(),
        "tablinq_conversation_id": conversation_id.clone(),
        "session_id": summary.session_id,
        "label": summary.label,
        "tab_url": summary.tab_url,
        "last_prompt": summary.last_prompt,
        "last_response": summary.last_response,
        "last_seen_at": summary.last_seen_at,
        "routes": conversation_route_hints_object(&conversation_id, None),
    })
}

pub(crate) fn conversation_selection_object(selection: ConversationSelection) -> Value {
    let conversation_id = selection.conversation_id.clone();
    json!({
        "conversation_id": conversation_id.clone(),
        "tablinq_conversation_id": selection.conversation_id,
        "session_id": selection.session_id,
        "current_url": selection.current_url,
        "browser": selection.browser,
    })
}

pub(crate) fn conversation_detail_object(detail: ConversationDetail) -> Value {
    let conversation_id = detail.summary.conversation_id.clone();
    let mut value = serde_json::to_value(detail).unwrap_or_else(|_| json!({}));
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "tablinq_conversation_id".to_owned(),
            Value::String(conversation_id),
        );
    }
    value
}

pub(crate) fn conversation_snapshot_object(snapshot: ConversationSnapshot) -> Value {
    let mut value = serde_json::to_value(snapshot).unwrap_or_else(|_| json!({}));
    if let Some(object) = value.as_object_mut()
        && let Some(conversation) = object
            .get_mut("conversation")
            .and_then(Value::as_object_mut)
        && let Some(conversation_id) = conversation
            .get("conversation_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    {
        conversation.insert(
            "tablinq_conversation_id".to_owned(),
            Value::String(conversation_id),
        );
    }
    value
}

pub(crate) fn staged_file_object(metadata: StagedFileMetadata) -> Value {
    json!({
        "id": metadata.file_id,
        "object": "tablinq.file",
        "file_name": metadata.file_name,
        "content_type": metadata.content_type,
        "size_bytes": metadata.size_bytes,
        "sha256": metadata.sha256,
        "created_at": metadata.created_at,
        "expires_at": metadata.expires_at,
        "linked_request_ids": metadata.linked_request_ids,
        "linked_conversation_ids": metadata.linked_conversation_ids,
    })
}

pub(crate) fn artifact_object(core: &CoreService, artifact: ArtifactSummary) -> Result<Value> {
    let linked_job_ids = core.artifact_job_ids(&artifact.artifact_id)?;
    let linked_request_ids = core.artifact_request_ids(&artifact.artifact_id)?;
    Ok(json!({
        "id": artifact.artifact_id,
        "object": "tablinq.artifact",
        "file_name": artifact.file_name,
        "content_type": artifact.content_type,
        "size_bytes": artifact.size_bytes,
        "sha256": artifact.sha256,
        "conversation_id": artifact.conversation_id,
        "message_id": artifact.message_id,
        "created_at": artifact.created_at,
        "linked_job_ids": linked_job_ids,
        "linked_request_ids": linked_request_ids,
        "content_url": format!("{V1_ARTIFACTS_PATH}/{}/content", artifact.artifact_id),
    }))
}

pub(crate) fn conversation_not_found_response(
    request_id: &str,
    conversation_id: &str,
) -> HttpResponse {
    json_response(
        404,
        json!({
            "request_id": request_id,
            "error": "conversation_not_found",
            "message": format!("conversation {conversation_id} was not found"),
        }),
    )
}

pub(crate) fn conversation_route_hints_object(
    conversation_id: &str,
    latest_request_id: Option<&str>,
) -> Value {
    json!({
        "detail": format!("/v1/conversations/{conversation_id}"),
        "messages": format!("/v1/conversations/{conversation_id}/messages"),
        "snapshot": format!("/v1/conversations/{conversation_id}/snapshot"),
        "select": V1_CONVERSATIONS_SELECT_PATH,
        "latest_request": latest_request_id.map(|request_id| format!("/v1/requests/{request_id}")),
        "latest_request_evidence": latest_request_id.map(|request_id| format!("/v1/requests/{request_id}/evidence")),
    })
}
