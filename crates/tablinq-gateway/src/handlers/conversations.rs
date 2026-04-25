use crate::http::{HttpResponse, RequestContext};
use crate::json::{
    conversation_detail_object, conversation_not_found_response, conversation_selection_object,
    conversation_snapshot_object, conversation_summary_object, json_response,
};
use crate::routes::{
    conversation_id_from_path, conversation_messages_id_from_path,
    conversation_snapshot_id_from_path, parse_conversation_message_query,
    session_conversations_id_from_path,
};
use anyhow::Result;
use gptd::CoreService;
use serde_json::json;
use shared_protocol::ConversationSelectCommand;

pub(crate) fn handle_session_conversations_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(session_id) = session_conversations_id_from_path(&context.request.path) else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };
    let Some(session_conversations) = core.list_session_conversations(session_id)? else {
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
            "object": "tablinq.session_conversation_list",
            "request_id": context.request_id,
            "session": session_conversations.session,
            "data": session_conversations
                .conversations
                .into_iter()
                .map(conversation_summary_object)
                .collect::<Vec<_>>(),
        }),
    ))
}

pub(crate) fn handle_conversations_request(
    core: &CoreService,
    request_id: &str,
) -> Result<HttpResponse> {
    let conversations = core.list_conversations()?;
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.conversation_list",
            "request_id": request_id,
            "data": conversations
                .conversations
                .into_iter()
                .map(conversation_summary_object)
                .collect::<Vec<_>>(),
        }),
    ))
}

pub(crate) fn handle_conversation_detail_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(conversation_id) = conversation_id_from_path(&context.request.path) else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };
    let Some(detail) = core.find_conversation_detail(conversation_id)? else {
        return Ok(conversation_not_found_response(
            &context.request_id,
            conversation_id,
        ));
    };
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.conversation",
            "request_id": context.request_id,
            "data": conversation_detail_object(detail),
        }),
    ))
}

pub(crate) fn handle_conversation_messages_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(conversation_id) = conversation_messages_id_from_path(&context.request.path) else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };
    let query = match parse_conversation_message_query(&context.request.path, &context.request_id) {
        Ok(query) => query,
        Err(response) => return Ok(response),
    };
    let Some(history) =
        core.list_conversation_history(conversation_id, query.limit, query.offset)?
    else {
        return Ok(conversation_not_found_response(
            &context.request_id,
            conversation_id,
        ));
    };
    Ok(json_response(200, {
        let conversation_id = history.conversation_id.clone();
        json!({
            "object": "tablinq.conversation_message_list",
            "request_id": context.request_id,
            "conversation_id": conversation_id.clone(),
            "tablinq_conversation_id": conversation_id,
            "data": history.messages,
            "pagination": {
                "limit": history.limit,
                "offset": history.offset,
                "next_offset": history.next_offset,
                "has_more": history.next_offset.is_some(),
                "total_items": history.total_items,
            },
        })
    }))
}

pub(crate) fn handle_conversation_snapshot_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(conversation_id) = conversation_snapshot_id_from_path(&context.request.path) else {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ));
    };
    let Some(snapshot) = core.conversation_snapshot(conversation_id)? else {
        return Ok(conversation_not_found_response(
            &context.request_id,
            conversation_id,
        ));
    };
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.conversation_snapshot",
            "request_id": context.request_id,
            "data": conversation_snapshot_object(snapshot),
        }),
    ))
}

pub(crate) fn handle_conversation_select_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let command = match serde_json::from_slice::<ConversationSelectCommand>(&context.request.body) {
        Ok(command) => command,
        Err(err) => {
            return Ok(json_response(
                400,
                json!({
                    "request_id": context.request_id,
                    "error": "invalid_request",
                    "message": format!("invalid JSON body: {err}"),
                }),
            ));
        }
    };
    if let Err(err) = command.validate() {
        return Ok(json_response(
            400,
            json!({
                "request_id": context.request_id,
                "error": "invalid_request",
                "message": format!("{err}"),
            }),
        ));
    }
    if let Some(conversation_id) = command.conversation_id.as_deref()
        && core.find_conversation(conversation_id)?.is_none()
    {
        return Ok(json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "conversation_not_found",
                "message": format!("conversation {conversation_id} was not found"),
            }),
        ));
    }

    match core.select_conversation(command) {
        Ok(selection) => Ok(json_response(
            200,
            json!({
                "object": "tablinq.conversation_selection",
                "request_id": context.request_id,
                "data": conversation_selection_object(selection),
            }),
        )),
        Err(err) => {
            let message = format!("{err:#}");
            let (status, error) =
                if message.contains("was not found") && message.contains("conversation") {
                    (404, "conversation_not_found")
                } else {
                    (400, "invalid_request")
                };
            Ok(json_response(
                status,
                json!({
                    "request_id": context.request_id,
                    "error": error,
                    "message": message,
                }),
            ))
        }
    }
}
