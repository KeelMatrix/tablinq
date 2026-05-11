use crate::http::{HttpResponse, RequestContext};
use crate::json::{
    api_error_response, compact_conversation_summary_object, conversation_detail_object,
    conversation_not_found_response, conversation_selection_object, conversation_snapshot_object,
    conversation_summary_object, json_response, pagination_object, response_meta_object,
};
use crate::routes::{
    PaginationQuery, REQUEST_LIST_MAX_LIMIT, conversation_id_from_path,
    conversation_messages_id_from_path, conversation_snapshot_id_from_path, includes_value,
    paginate_items, parse_conversation_detail_query, parse_conversation_message_query,
    parse_snapshot_query, parse_state_list_query, session_conversations_id_from_path,
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
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            "session was not found",
        ));
    };
    let Some(session_conversations) = core.list_session_conversations(session_id)? else {
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            format!("session {session_id} was not found"),
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
    context: &RequestContext,
) -> Result<HttpResponse> {
    let query = match parse_state_list_query(&context.request.path, &context.request_id, &[]) {
        Ok(query) => query,
        Err(response) => return Ok(response),
    };
    let conversations = core.list_conversations()?;
    let compact = query.wants_summary();
    let page = paginate_items(conversations.conversations, query.pagination);
    let truncated_fields = if compact {
        vec!["last_prompt", "last_response", "tab_url"]
    } else {
        vec![]
    };
    let pagination = pagination_object(&page);
    let data = page
        .items
        .into_iter()
        .map(|summary| {
            if compact {
                compact_conversation_summary_object(summary)
            } else {
                conversation_summary_object(summary)
            }
        })
        .collect::<Vec<_>>();
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.conversation_list",
            "request_id": context.request_id,
            "data": data,
            "pagination": pagination,
            "meta": response_meta_object(query.view, query.compact, truncated_fields),
        }),
    ))
}

pub(crate) fn handle_conversation_detail_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(conversation_id) = conversation_id_from_path(&context.request.path) else {
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            "conversation was not found",
        ));
    };
    let query = match parse_conversation_detail_query(&context.request.path, &context.request_id) {
        Ok(query) => query,
        Err(response) => return Ok(response),
    };
    let Some(detail) = core.find_conversation_detail(conversation_id)? else {
        return Ok(conversation_not_found_response(
            &context.request_id,
            conversation_id,
        ));
    };
    let mut data = if query.view.is_summary() || query.compact {
        compact_conversation_summary_object(detail.summary)
    } else {
        conversation_detail_object(detail)
    };
    let mut truncated_fields = if query.view.is_summary() || query.compact {
        vec!["session", "recovery_manifest", "counts"]
    } else {
        vec![]
    };
    if includes_value(&query.includes, "messages")
        && let Some(history) = core.list_conversation_history(conversation_id, 50, 0)?
        && let Some(object) = data.as_object_mut()
    {
        let returned = history.messages.len();
        let limit = history.limit;
        let offset = history.offset;
        let next_offset = history.next_offset;
        let total_items = history.total_items;
        let messages = history.messages;
        object.insert(
            "messages".to_owned(),
            json!({
                "data": messages,
                "pagination": {
                    "limit": limit,
                    "offset": offset,
                    "returned": returned,
                    "next_offset": next_offset,
                    "has_more": next_offset.is_some(),
                    "total_items": total_items,
                }
            }),
        );
    }
    if includes_value(&query.includes, "snapshot")
        && let Some(snapshot) = core.conversation_snapshot(conversation_id)?
        && let Some(object) = data.as_object_mut()
    {
        object.insert(
            "snapshot".to_owned(),
            conversation_snapshot_object(snapshot),
        );
    }
    if query.compact && !includes_value(&query.includes, "artifacts") {
        truncated_fields.push("artifacts");
    }
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.conversation",
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

pub(crate) fn handle_conversation_messages_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(conversation_id) = conversation_messages_id_from_path(&context.request.path) else {
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            "conversation was not found",
        ));
    };
    let query = match parse_conversation_message_query(&context.request.path, &context.request_id) {
        Ok(query) => query,
        Err(response) => return Ok(response),
    };
    let history_limit = if query.order == crate::routes::MessageOrder::Desc {
        REQUEST_LIST_MAX_LIMIT
    } else {
        query.limit
    };
    let history_offset = if query.order == crate::routes::MessageOrder::Desc {
        0
    } else {
        query.offset
    };
    let Some(mut history) =
        core.list_conversation_history(conversation_id, history_limit, history_offset)?
    else {
        return Ok(conversation_not_found_response(
            &context.request_id,
            conversation_id,
        ));
    };
    if query.order == crate::routes::MessageOrder::Desc {
        history.messages.reverse();
        let page = paginate_items(
            history.messages,
            PaginationQuery {
                limit: Some(query.limit),
                offset: query.offset,
            },
        );
        history.messages = page.items;
        history.limit = query.limit;
        history.offset = query.offset;
        history.next_offset = page.next_offset;
        history.total_items = page.total;
    }
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
                "returned": history.messages.len(),
            },
            "meta": {
                "order": query.order.as_str(),
                "view": "detail",
                "compact": false,
                "truncated": false,
                "truncated_fields": [],
            },
        })
    }))
}

pub(crate) fn handle_conversation_snapshot_request(
    core: &CoreService,
    context: &RequestContext,
) -> Result<HttpResponse> {
    let Some(conversation_id) = conversation_snapshot_id_from_path(&context.request.path) else {
        return Ok(api_error_response(
            404,
            &context.request_id,
            "not_found_error",
            "not_found",
            "conversation was not found",
        ));
    };
    let query = match parse_snapshot_query(&context.request.path, &context.request_id) {
        Ok(query) => query,
        Err(response) => return Ok(response),
    };
    let Some(snapshot) = core.conversation_snapshot(conversation_id)? else {
        return Ok(conversation_not_found_response(
            &context.request_id,
            conversation_id,
        ));
    };
    let mut data = conversation_snapshot_object(snapshot);
    let mut truncated_fields = Vec::new();
    let original_recent_message_count = data
        .get("recent_messages")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    let max_items =
        if query.compact && !query.view.is_full() && !includes_value(&query.includes, "messages") {
            Some(3usize)
        } else {
            None
        };
    if let Some(max_items) = max_items
        && let Some(messages) = data
            .get_mut("recent_messages")
            .and_then(serde_json::Value::as_array_mut)
        && messages.len() > max_items
    {
        messages.truncate(max_items);
        truncated_fields.push("recent_messages");
    }
    if query.compact
        && !query.view.is_full()
        && !includes_value(&query.includes, "raw")
        && let Some(object) = data.as_object_mut()
    {
        if !includes_value(&query.includes, "debug") {
            if object.remove("latest_recovery_packet").is_some() {
                truncated_fields.push("latest_recovery_packet");
            }
            if object.remove("latest_recovery_trace").is_some() {
                truncated_fields.push("latest_recovery_trace");
            }
            if object.remove("recovery_manifest").is_some() {
                truncated_fields.push("recovery_manifest");
            }
        }
        if !includes_value(&query.includes, "artifacts")
            && object.remove("tracked_generated_artifacts").is_some()
        {
            truncated_fields.push("tracked_generated_artifacts");
        }
    }
    let returned_items = data
        .get("recent_messages")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0);
    Ok(json_response(
        200,
        json!({
            "object": "tablinq.conversation_snapshot",
            "request_id": context.request_id,
            "data": data,
            "meta": {
                "view": query.view.as_str(),
                "compact": query.compact,
                "include": query.includes,
                "truncated": !truncated_fields.is_empty(),
                "truncated_fields": truncated_fields,
                "max_items": max_items,
                "returned_items": returned_items,
                "has_more": max_items.is_some_and(|limit| original_recent_message_count > limit),
            },
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
            return Ok(api_error_response(
                400,
                &context.request_id,
                "invalid_request_error",
                "invalid_request_json",
                format!("invalid JSON body: {err}"),
            ));
        }
    };
    if let Err(err) = command.validate() {
        return Ok(api_error_response(
            400,
            &context.request_id,
            "invalid_request_error",
            "invalid_request",
            format!("{err}"),
        ));
    }
    if let Some(conversation_id) = command.conversation_id.as_deref()
        && core.find_conversation(conversation_id)?.is_none()
    {
        return Ok(conversation_not_found_response(
            &context.request_id,
            conversation_id,
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
            let (status, error_type, code) =
                if message.contains("was not found") && message.contains("conversation") {
                    (404, "not_found_error", "conversation_not_found")
                } else {
                    (400, "invalid_request_error", "invalid_request")
                };
            Ok(api_error_response(
                status,
                &context.request_id,
                error_type,
                code,
                message,
            ))
        }
    }
}
