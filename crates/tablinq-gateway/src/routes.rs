use crate::http::HttpResponse;
use crate::json::api_error_response;
use shared_protocol::JobStatus;

pub(crate) const HEALTH_PATH: &str = "/health";
pub(crate) const READY_PATH: &str = "/ready";
pub(crate) const ADMIN_STATUS_PATH: &str = "/admin/status";
pub(crate) const ADMIN_SHUTDOWN_PATH: &str = "/admin/shutdown";
pub(crate) const ADMIN_CLEAR_PATH: &str = "/admin/clear";
pub(crate) const V1_MODELS_PATH: &str = "/v1/models";
pub(crate) const V1_CAPABILITIES_PATH: &str = "/v1/capabilities";
pub(crate) const V1_CHAT_COMPLETIONS_PATH: &str = "/v1/chat/completions";
pub(crate) const V1_FILES_PATH: &str = "/v1/files";
pub(crate) const V1_ARTIFACTS_PATH: &str = "/v1/artifacts";
pub(crate) const V1_PROVIDER_PATH: &str = "/v1/provider";
pub(crate) const V1_STATUS_PATH: &str = "/v1/status";
pub(crate) const V1_DOCTOR_PATH: &str = "/v1/doctor";
pub(crate) const V1_CONFIG_PATH: &str = "/v1/config";
pub(crate) const V1_JOBS_PATH: &str = "/v1/jobs";
pub(crate) const V1_SESSIONS_PATH: &str = "/v1/sessions";
pub(crate) const V1_REQUESTS_PATH: &str = "/v1/requests";
pub(crate) const V1_CONVERSATIONS_PATH: &str = "/v1/conversations";
pub(crate) const V1_CONVERSATIONS_SELECT_PATH: &str = "/v1/conversations/select";
pub(crate) const V1_USAGE_PATH: &str = "/v1/usage";
pub(crate) const V1_BROWSER_ATTACH_PATH: &str = "/v1/browser/attach";
pub(crate) const V1_BROWSER_DETACH_PATH: &str = "/v1/browser/detach";
pub(crate) const V1_BROWSER_PROFILE_INFO_PATH: &str = "/v1/browser/profile-info";
pub(crate) const V1_BROWSER_CONNECTION_INFO_PATH: &str = "/v1/browser/connection-info";
pub(crate) const V1_SESSIONS_PREFIX: &str = "/v1/sessions/";
pub(crate) const V1_JOBS_PREFIX: &str = "/v1/jobs/";
pub(crate) const V1_CONVERSATIONS_PREFIX: &str = "/v1/conversations/";
pub(crate) const V1_REQUESTS_PREFIX: &str = "/v1/requests/";
pub(crate) const REQUEST_SUMMARY_LIMIT: usize = 50;
pub(crate) const REQUEST_LIST_MAX_LIMIT: usize = 100;
pub(crate) const CONVERSATION_MESSAGE_LIST_DEFAULT_LIMIT: usize = 50;
pub(crate) const STATE_LIST_DEFAULT_LIMIT: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Route {
    Health,
    Ready,
    AdminStatus,
    AdminShutdown,
    AdminClear,
    Models,
    Capabilities,
    Provider,
    Status,
    Doctor,
    Config,
    Jobs,
    JobDetail,
    JobCancel,
    Usage,
    BrowserAttach,
    BrowserDetach,
    BrowserProfileInfo,
    BrowserConnectionInfo,
    Sessions,
    SessionConversations,
    Requests,
    Conversations,
    ConversationMessages,
    ConversationSnapshot,
    ConversationDetail,
    RequestEvidence,
    RequestSummary,
    FilesList,
    FileUpload,
    FileDetail,
    ArtifactsList,
    ArtifactDetail,
    RequestAction,
    ConversationSelect,
    ChatCompletions,
    NotFound,
}

pub(crate) fn match_route(method: &str, path: &str) -> Route {
    let request_path = normalized_request_path(path);
    match (method, request_path) {
        ("GET", HEALTH_PATH) => Route::Health,
        ("GET", READY_PATH) => Route::Ready,
        ("GET", ADMIN_STATUS_PATH) => Route::AdminStatus,
        ("POST", ADMIN_SHUTDOWN_PATH) => Route::AdminShutdown,
        ("POST", ADMIN_CLEAR_PATH) => Route::AdminClear,
        ("GET", V1_MODELS_PATH) => Route::Models,
        ("GET", V1_CAPABILITIES_PATH) => Route::Capabilities,
        ("GET", V1_PROVIDER_PATH) => Route::Provider,
        ("GET", V1_STATUS_PATH) => Route::Status,
        ("GET", V1_DOCTOR_PATH) => Route::Doctor,
        ("GET", V1_CONFIG_PATH) => Route::Config,
        ("GET", V1_JOBS_PATH) => Route::Jobs,
        ("GET", V1_USAGE_PATH) => Route::Usage,
        ("POST", V1_BROWSER_ATTACH_PATH) => Route::BrowserAttach,
        ("POST", V1_BROWSER_DETACH_PATH) => Route::BrowserDetach,
        ("GET", V1_BROWSER_PROFILE_INFO_PATH) => Route::BrowserProfileInfo,
        ("GET", V1_BROWSER_CONNECTION_INFO_PATH) => Route::BrowserConnectionInfo,
        ("GET", V1_SESSIONS_PATH) => Route::Sessions,
        ("GET", V1_REQUESTS_PATH) => Route::Requests,
        ("GET", V1_CONVERSATIONS_PATH) => Route::Conversations,
        ("GET", V1_FILES_PATH) => Route::FilesList,
        ("POST", V1_FILES_PATH) => Route::FileUpload,
        ("GET", V1_ARTIFACTS_PATH) => Route::ArtifactsList,
        ("POST", V1_CONVERSATIONS_SELECT_PATH) => Route::ConversationSelect,
        ("POST", V1_CHAT_COMPLETIONS_PATH) => Route::ChatCompletions,
        ("GET", _) if job_id_from_path(path).is_some() => Route::JobDetail,
        ("POST", _) if cancel_job_id_from_path(path).is_some() => Route::JobCancel,
        ("GET", _) if session_conversations_id_from_path(path).is_some() => {
            Route::SessionConversations
        }
        ("GET", _) if conversation_messages_id_from_path(path).is_some() => {
            Route::ConversationMessages
        }
        ("GET", _) if conversation_snapshot_id_from_path(path).is_some() => {
            Route::ConversationSnapshot
        }
        ("GET", _) if conversation_id_from_path(path).is_some() => Route::ConversationDetail,
        ("GET", _) if request_evidence_request_id_from_path(path).is_some() => {
            Route::RequestEvidence
        }
        ("GET", _) if request_id_from_path(path).is_some() => Route::RequestSummary,
        ("GET", _) if path.starts_with(&format!("{V1_FILES_PATH}/")) => Route::FileDetail,
        ("GET", _) if path.starts_with(&format!("{V1_ARTIFACTS_PATH}/")) => Route::ArtifactDetail,
        ("POST", _) if request_path.starts_with(V1_REQUESTS_PREFIX) => Route::RequestAction,
        _ => Route::NotFound,
    }
}

pub(crate) fn method_not_allowed_for_path(path: &str) -> Option<&'static str> {
    match normalized_request_path(path) {
        V1_MODELS_PATH => Some("GET"),
        V1_CAPABILITIES_PATH => Some("GET"),
        V1_PROVIDER_PATH => Some("GET"),
        V1_STATUS_PATH => Some("GET"),
        V1_DOCTOR_PATH => Some("GET"),
        V1_CONFIG_PATH => Some("GET"),
        V1_JOBS_PATH => Some("GET"),
        V1_SESSIONS_PATH => Some("GET"),
        V1_REQUESTS_PATH => Some("GET"),
        V1_CONVERSATIONS_PATH => Some("GET"),
        V1_USAGE_PATH => Some("GET"),
        V1_CHAT_COMPLETIONS_PATH => Some("POST"),
        V1_FILES_PATH => Some("GET, POST"),
        V1_ARTIFACTS_PATH => Some("GET"),
        V1_CONVERSATIONS_SELECT_PATH => Some("POST"),
        V1_BROWSER_ATTACH_PATH => Some("POST"),
        V1_BROWSER_DETACH_PATH => Some("POST"),
        V1_BROWSER_PROFILE_INFO_PATH => Some("GET"),
        V1_BROWSER_CONNECTION_INFO_PATH => Some("GET"),
        _ => {
            if cancel_job_id_from_path(path).is_some()
                || cancel_request_id_from_path(path).is_some()
            {
                Some("POST")
            } else if job_id_from_path(path).is_some()
                || request_id_from_path(path).is_some()
                || request_evidence_request_id_from_path(path).is_some()
                || session_conversations_id_from_path(path).is_some()
                || conversation_messages_id_from_path(path).is_some()
                || conversation_snapshot_id_from_path(path).is_some()
                || conversation_id_from_path(path).is_some()
                || file_id_from_path(path).is_some()
                || artifact_id_from_path(path).is_some()
            {
                Some("GET")
            } else {
                None
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResponseView {
    Summary,
    Detail,
    Full,
}

impl ResponseView {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Detail => "detail",
            Self::Full => "full",
        }
    }

    pub(crate) fn is_summary(self) -> bool {
        matches!(self, Self::Summary)
    }

    pub(crate) fn is_full(self) -> bool {
        matches!(self, Self::Full)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PaginationQuery {
    pub(crate) limit: Option<usize>,
    pub(crate) offset: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct PaginatedItems<T> {
    pub(crate) items: Vec<T>,
    pub(crate) limit: Option<usize>,
    pub(crate) offset: usize,
    pub(crate) returned: usize,
    pub(crate) has_more: bool,
    pub(crate) next_offset: Option<usize>,
    pub(crate) total: usize,
}

#[derive(Debug, Clone)]
pub(crate) struct StateListQuery {
    pub(crate) pagination: PaginationQuery,
    pub(crate) view: ResponseView,
    pub(crate) compact: bool,
    pub(crate) includes: Vec<String>,
}

impl StateListQuery {
    pub(crate) fn wants_summary(&self) -> bool {
        self.view.is_summary() || self.compact
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RequestListQuery {
    pub(crate) pagination: PaginationQuery,
    pub(crate) status: Option<JobStatus>,
    pub(crate) view: ResponseView,
    pub(crate) compact: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ConversationMessageQuery {
    pub(crate) limit: usize,
    pub(crate) offset: usize,
    pub(crate) order: MessageOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MessageOrder {
    Asc,
    Desc,
}

impl MessageOrder {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RequestDetailQuery {
    pub(crate) view: ResponseView,
    pub(crate) includes: Vec<String>,
    pub(crate) compact: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct SnapshotQuery {
    pub(crate) view: ResponseView,
    pub(crate) includes: Vec<String>,
    pub(crate) compact: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct UsageQuery {
    pub(crate) pagination: PaginationQuery,
    pub(crate) view: ResponseView,
    pub(crate) summary: bool,
}

pub(crate) fn parse_request_list_query(
    path: &str,
    request_id: &str,
) -> std::result::Result<RequestListQuery, HttpResponse> {
    let mut query = RequestListQuery {
        pagination: PaginationQuery {
            limit: Some(REQUEST_SUMMARY_LIMIT),
            offset: 0,
        },
        status: None,
        view: ResponseView::Detail,
        compact: false,
    };
    for (key, value) in request_query_pairs(path) {
        match key {
            "limit" => {
                query.pagination.limit = Some(parse_limit(value, request_id)?);
            }
            "offset" => {
                query.pagination.offset = parse_offset(value, request_id)?;
            }
            "status" => {
                query.status = Some(value.parse::<JobStatus>().map_err(|_| {
                    api_error_response(
                        400,
                        request_id,
                        "invalid_request_error",
                        "invalid_request",
                        format!("unsupported status filter: {value}"),
                    )
                })?);
            }
            "view" => {
                query.view = parse_view(value, request_id)?;
            }
            "compact" => {
                query.compact = parse_bool(value, "compact", request_id)?;
            }
            "include" | "summary" | "order" => {
                return Err(unsupported_query_param_response(request_id, key));
            }
            _ => {}
        }
    }
    Ok(query)
}

pub(crate) fn parse_conversation_message_query(
    path: &str,
    request_id: &str,
) -> std::result::Result<ConversationMessageQuery, HttpResponse> {
    let mut query = ConversationMessageQuery {
        limit: CONVERSATION_MESSAGE_LIST_DEFAULT_LIMIT,
        offset: 0,
        order: MessageOrder::Asc,
    };
    for (key, value) in request_query_pairs(path) {
        match key {
            "limit" => {
                query.limit = parse_limit(value, request_id)?;
            }
            "offset" => {
                query.offset = parse_offset(value, request_id)?;
            }
            "order" => {
                query.order = parse_message_order(value, request_id)?;
            }
            "view" | "include" | "compact" | "summary" => {
                return Err(unsupported_query_param_response(request_id, key));
            }
            _ => {}
        }
    }
    Ok(query)
}

pub(crate) fn parse_state_list_query(
    path: &str,
    request_id: &str,
    allowed_includes: &[&str],
) -> std::result::Result<StateListQuery, HttpResponse> {
    let mut query = StateListQuery {
        pagination: PaginationQuery {
            limit: None,
            offset: 0,
        },
        view: ResponseView::Detail,
        compact: false,
        includes: vec![],
    };
    let mut saw_explicit_shape = false;
    for (key, value) in request_query_pairs(path) {
        match key {
            "limit" => {
                query.pagination.limit = Some(parse_limit(value, request_id)?);
                saw_explicit_shape = true;
            }
            "offset" => {
                query.pagination.offset = parse_offset(value, request_id)?;
                saw_explicit_shape = true;
            }
            "view" => {
                query.view = parse_view(value, request_id)?;
                saw_explicit_shape = true;
            }
            "compact" => {
                query.compact = parse_bool(value, "compact", request_id)?;
                saw_explicit_shape = true;
            }
            "include" => {
                append_includes(&mut query.includes, value, allowed_includes, request_id)?;
                saw_explicit_shape = true;
            }
            "summary" | "order" => {
                return Err(unsupported_query_param_response(request_id, key));
            }
            _ => {}
        }
    }
    if query.pagination.limit.is_none()
        && (query.pagination.offset > 0 || query.view.is_summary() || query.compact)
    {
        query.pagination.limit = Some(STATE_LIST_DEFAULT_LIMIT);
    }
    if !saw_explicit_shape {
        query.view = ResponseView::Detail;
    }
    Ok(query)
}

pub(crate) fn parse_request_detail_query(
    path: &str,
    request_id: &str,
) -> std::result::Result<RequestDetailQuery, HttpResponse> {
    let mut query = RequestDetailQuery {
        view: ResponseView::Detail,
        includes: vec![],
        compact: false,
    };
    for (key, value) in request_query_pairs(path) {
        match key {
            "view" => query.view = parse_view(value, request_id)?,
            "include" => append_includes(
                &mut query.includes,
                value,
                &["evidence", "debug", "raw"],
                request_id,
            )?,
            "compact" => query.compact = parse_bool(value, "compact", request_id)?,
            "limit" | "offset" | "summary" | "order" => {
                return Err(unsupported_query_param_response(request_id, key));
            }
            _ => {}
        }
    }
    Ok(query)
}

pub(crate) fn parse_request_evidence_query(
    path: &str,
    request_id: &str,
) -> std::result::Result<PaginationQuery, HttpResponse> {
    let mut query = PaginationQuery {
        limit: Some(1),
        offset: 0,
    };
    for (key, value) in request_query_pairs(path) {
        match key {
            "limit" => query.limit = Some(parse_limit(value, request_id)?),
            "offset" => query.offset = parse_offset(value, request_id)?,
            "view" | "include" | "compact" | "summary" | "order" => {
                return Err(unsupported_query_param_response(request_id, key));
            }
            _ => {}
        }
    }
    Ok(query)
}

pub(crate) fn parse_snapshot_query(
    path: &str,
    request_id: &str,
) -> std::result::Result<SnapshotQuery, HttpResponse> {
    parse_expansion_query(path, request_id, &["messages", "artifacts", "debug", "raw"])
}

pub(crate) fn parse_conversation_detail_query(
    path: &str,
    request_id: &str,
) -> std::result::Result<SnapshotQuery, HttpResponse> {
    parse_expansion_query(
        path,
        request_id,
        &["messages", "snapshot", "artifacts", "debug", "raw"],
    )
}

fn parse_expansion_query(
    path: &str,
    request_id: &str,
    allowed_includes: &[&str],
) -> std::result::Result<SnapshotQuery, HttpResponse> {
    let mut query = SnapshotQuery {
        view: ResponseView::Detail,
        includes: vec![],
        compact: false,
    };
    for (key, value) in request_query_pairs(path) {
        match key {
            "view" => query.view = parse_view(value, request_id)?,
            "include" => append_includes(&mut query.includes, value, allowed_includes, request_id)?,
            "compact" => query.compact = parse_bool(value, "compact", request_id)?,
            "limit" | "offset" | "summary" | "order" => {
                return Err(unsupported_query_param_response(request_id, key));
            }
            _ => {}
        }
    }
    Ok(query)
}

pub(crate) fn parse_usage_query(
    path: &str,
    request_id: &str,
) -> std::result::Result<UsageQuery, HttpResponse> {
    let mut query = UsageQuery {
        pagination: PaginationQuery {
            limit: Some(REQUEST_SUMMARY_LIMIT),
            offset: 0,
        },
        view: ResponseView::Detail,
        summary: false,
    };
    for (key, value) in request_query_pairs(path) {
        match key {
            "limit" => query.pagination.limit = Some(parse_limit(value, request_id)?),
            "offset" => query.pagination.offset = parse_offset(value, request_id)?,
            "view" => query.view = parse_view(value, request_id)?,
            "summary" => query.summary = parse_bool(value, "summary", request_id)?,
            "include" | "compact" | "order" => {
                return Err(unsupported_query_param_response(request_id, key));
            }
            _ => {}
        }
    }
    Ok(query)
}

pub(crate) fn paginate_items<T>(items: Vec<T>, pagination: PaginationQuery) -> PaginatedItems<T> {
    let total = items.len();
    let start = pagination.offset.min(total);
    let page_limit = pagination
        .limit
        .unwrap_or_else(|| total.saturating_sub(start));
    let end = start.saturating_add(page_limit).min(total);
    let take = end.saturating_sub(start);
    let page_items = items.into_iter().skip(start).take(take).collect::<Vec<_>>();
    let returned = page_items.len();
    let has_more = end < total;
    PaginatedItems {
        items: page_items,
        limit: pagination.limit,
        offset: pagination.offset,
        returned,
        has_more,
        next_offset: has_more.then_some(end),
        total,
    }
}

pub(crate) fn includes_value(includes: &[String], value: &str) -> bool {
    includes.iter().any(|item| item == value)
}

fn parse_limit(value: &str, request_id: &str) -> std::result::Result<usize, HttpResponse> {
    let parsed = value.parse::<usize>().map_err(|_| {
        api_error_response(
            400,
            request_id,
            "invalid_request_error",
            "invalid_request",
            "limit must be a positive integer",
        )
    })?;
    if parsed == 0 || parsed > REQUEST_LIST_MAX_LIMIT {
        return Err(api_error_response(
            400,
            request_id,
            "invalid_request_error",
            "invalid_request",
            format!("limit must be between 1 and {REQUEST_LIST_MAX_LIMIT}"),
        ));
    }
    Ok(parsed)
}

fn parse_offset(value: &str, request_id: &str) -> std::result::Result<usize, HttpResponse> {
    value.parse::<usize>().map_err(|_| {
        api_error_response(
            400,
            request_id,
            "invalid_request_error",
            "invalid_request",
            "offset must be a non-negative integer",
        )
    })
}

fn parse_bool(
    value: &str,
    name: &str,
    request_id: &str,
) -> std::result::Result<bool, HttpResponse> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(api_error_response(
            400,
            request_id,
            "invalid_request_error",
            "invalid_request",
            format!("{name} must be true or false"),
        )),
    }
}

fn parse_view(value: &str, request_id: &str) -> std::result::Result<ResponseView, HttpResponse> {
    match value {
        "summary" => Ok(ResponseView::Summary),
        "detail" => Ok(ResponseView::Detail),
        "full" => Ok(ResponseView::Full),
        _ => Err(api_error_response(
            400,
            request_id,
            "invalid_request_error",
            "invalid_request",
            "view must be one of summary, detail, full",
        )),
    }
}

fn parse_message_order(
    value: &str,
    request_id: &str,
) -> std::result::Result<MessageOrder, HttpResponse> {
    match value {
        "asc" => Ok(MessageOrder::Asc),
        "desc" => Ok(MessageOrder::Desc),
        _ => Err(api_error_response(
            400,
            request_id,
            "invalid_request_error",
            "invalid_request",
            "order must be one of asc, desc",
        )),
    }
}

fn append_includes(
    includes: &mut Vec<String>,
    value: &str,
    allowed: &[&str],
    request_id: &str,
) -> std::result::Result<(), HttpResponse> {
    for include in value.split(',') {
        let include = include.trim();
        if include.is_empty() {
            return Err(api_error_response(
                400,
                request_id,
                "invalid_request_error",
                "invalid_request",
                "include must not be empty",
            ));
        }
        if !allowed.iter().any(|allowed| allowed == &include) {
            return Err(api_error_response(
                400,
                request_id,
                "invalid_request_error",
                "invalid_request",
                format!("unsupported include: {include}"),
            ));
        }
        if !includes.iter().any(|existing| existing == include) {
            includes.push(include.to_owned());
        }
    }
    Ok(())
}

fn unsupported_query_param_response(request_id: &str, key: &str) -> HttpResponse {
    api_error_response(
        400,
        request_id,
        "invalid_request_error",
        "invalid_request",
        format!("unsupported query parameter for this route: {key}"),
    )
}

pub(crate) fn normalized_request_path(path: &str) -> &str {
    path.split_once('?').map(|(base, _)| base).unwrap_or(path)
}

pub(crate) fn request_query_pairs(path: &str) -> impl Iterator<Item = (&str, &str)> {
    path.split_once('?')
        .map(|(_, query)| query)
        .into_iter()
        .flat_map(|query| query.split('&'))
        .filter_map(|pair| {
            if pair.is_empty() {
                None
            } else if let Some((key, value)) = pair.split_once('=') {
                Some((key, value))
            } else {
                Some((pair, ""))
            }
        })
}

pub(crate) fn file_id_from_path(path: &str) -> Option<&str> {
    normalized_request_path(path)
        .strip_prefix(&format!("{V1_FILES_PATH}/"))
        .filter(|value| !value.contains('/'))
}

pub(crate) fn artifact_id_from_path(path: &str) -> Option<(&str, bool)> {
    let suffix = normalized_request_path(path).strip_prefix(&format!("{V1_ARTIFACTS_PATH}/"))?;
    if let Some(artifact_id) = suffix.strip_suffix("/content") {
        if artifact_id.contains('/') || artifact_id.is_empty() {
            return None;
        }
        Some((artifact_id, true))
    } else if suffix.contains('/') || suffix.is_empty() {
        None
    } else {
        Some((suffix, false))
    }
}

pub(crate) fn session_conversations_id_from_path(path: &str) -> Option<&str> {
    let suffix = normalized_request_path(path).strip_prefix(V1_SESSIONS_PREFIX)?;
    suffix
        .strip_suffix("/conversations")
        .filter(|value| !value.is_empty() && !value.contains('/'))
}

pub(crate) fn cancel_job_id_from_path(path: &str) -> Option<&str> {
    let suffix = normalized_request_path(path).strip_prefix(V1_JOBS_PREFIX)?;
    suffix
        .strip_suffix("/cancel")
        .filter(|value| !value.is_empty() && !value.contains('/'))
}

pub(crate) fn job_id_from_path(path: &str) -> Option<&str> {
    let suffix = normalized_request_path(path).strip_prefix(V1_JOBS_PREFIX)?;
    if suffix.is_empty() || suffix.contains('/') {
        None
    } else {
        Some(suffix)
    }
}

pub(crate) fn conversation_messages_id_from_path(path: &str) -> Option<&str> {
    let suffix = normalized_request_path(path).strip_prefix(V1_CONVERSATIONS_PREFIX)?;
    suffix
        .strip_suffix("/messages")
        .filter(|value| !value.is_empty() && !value.contains('/'))
}

pub(crate) fn conversation_snapshot_id_from_path(path: &str) -> Option<&str> {
    let suffix = normalized_request_path(path).strip_prefix(V1_CONVERSATIONS_PREFIX)?;
    suffix
        .strip_suffix("/snapshot")
        .filter(|value| !value.is_empty() && !value.contains('/'))
}

pub(crate) fn conversation_id_from_path(path: &str) -> Option<&str> {
    let suffix = normalized_request_path(path).strip_prefix(V1_CONVERSATIONS_PREFIX)?;
    if suffix.is_empty() || suffix.contains('/') || suffix == "select" {
        None
    } else {
        Some(suffix)
    }
}

pub(crate) fn cancel_request_id_from_path(path: &str) -> Option<&str> {
    let suffix = normalized_request_path(path).strip_prefix(V1_REQUESTS_PREFIX)?;
    suffix.strip_suffix("/cancel")
}

pub(crate) fn request_evidence_request_id_from_path(path: &str) -> Option<&str> {
    let suffix = normalized_request_path(path).strip_prefix(V1_REQUESTS_PREFIX)?;
    suffix
        .strip_suffix("/evidence")
        .filter(|value| !value.is_empty())
}

pub(crate) fn request_id_from_path(path: &str) -> Option<&str> {
    let suffix = normalized_request_path(path).strip_prefix(V1_REQUESTS_PREFIX)?;
    if suffix.is_empty() || suffix.contains('/') {
        None
    } else {
        Some(suffix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_matching_centralizes_dynamic_gateway_paths() {
        assert_eq!(match_route("GET", "/health"), Route::Health);
        assert_eq!(match_route("POST", "/admin/clear"), Route::AdminClear);
        assert_eq!(
            match_route("GET", "/v1/conversations/conv-1/messages?limit=5"),
            Route::ConversationMessages
        );
        assert_eq!(
            match_route("GET", "/v1/artifacts/art-1/content"),
            Route::ArtifactDetail
        );
        assert_eq!(
            match_route("POST", "/v1/requests/req-1/cancel"),
            Route::RequestAction
        );
        assert_eq!(match_route("GET", V1_STATUS_PATH), Route::Status);
        assert_eq!(match_route("GET", "/v1/jobs/job-1"), Route::JobDetail);
        assert_eq!(
            match_route("POST", "/v1/jobs/job-1/cancel"),
            Route::JobCancel
        );
        assert_eq!(
            match_route("POST", V1_BROWSER_ATTACH_PATH),
            Route::BrowserAttach
        );
        assert_eq!(match_route("DELETE", "/v1/files/file-1"), Route::NotFound);
    }

    #[test]
    fn method_not_allowed_catalog_covers_known_gateway_paths() {
        assert_eq!(method_not_allowed_for_path(V1_MODELS_PATH), Some("GET"));
        assert_eq!(
            method_not_allowed_for_path(V1_CHAT_COMPLETIONS_PATH),
            Some("POST")
        );
        assert_eq!(
            method_not_allowed_for_path(V1_FILES_PATH),
            Some("GET, POST")
        );
        assert_eq!(
            method_not_allowed_for_path("/v1/requests/req-1/cancel"),
            Some("POST")
        );
        assert_eq!(
            method_not_allowed_for_path("/v1/conversations/conv-1/messages"),
            Some("GET")
        );
    }

    #[test]
    fn request_query_parsing_applies_defaults_filters_and_limits() {
        let query =
            parse_request_list_query("/v1/requests?limit=25&offset=5&status=running", "req")
                .expect("valid request query");
        assert_eq!(query.pagination.limit, Some(25));
        assert_eq!(query.pagination.offset, 5);
        assert_eq!(query.status, Some(JobStatus::Running));
        assert_eq!(query.view, ResponseView::Detail);

        let err = parse_request_list_query("/v1/requests?limit=0", "req")
            .expect_err("invalid limit should become HTTP response");
        assert_eq!(err.status_code, 400);
    }

    #[test]
    fn conversation_message_query_uses_default_limit() {
        let query = parse_conversation_message_query("/v1/conversations/conv-1/messages", "req")
            .expect("valid conversation message query");
        assert_eq!(query.limit, CONVERSATION_MESSAGE_LIST_DEFAULT_LIMIT);
        assert_eq!(query.offset, 0);
        assert_eq!(query.order, MessageOrder::Asc);
    }
}
