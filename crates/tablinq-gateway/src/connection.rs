use crate::auth::authorize_request;
use crate::capabilities::CapabilityCatalogCache;
use crate::handlers::{artifacts, chat, conversations, files, health_admin, metadata, requests};
use crate::http::{
    json_response, log_request, log_response, next_request_id, parse_http_request,
    write_http_response,
};
use crate::process_control::GatewayStatusSnapshot;
use crate::routes::{Route, match_route};
use crate::scheduler::GatewayScheduler;
use crate::uploads::UploadStore;
use anyhow::{Context, Result};
use gptd::logging::JsonLogger;
use gptd::{AppConfig, CoreService};
use serde_json::json;
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::time::Duration;

#[allow(clippy::too_many_arguments)]
pub(crate) fn handle_connection(
    mut stream: TcpStream,
    remote_addr: String,
    config: &AppConfig,
    core: &CoreService,
    snapshot: &GatewayStatusSnapshot,
    shutdown: &AtomicBool,
    request_counter: &AtomicU64,
    scheduler: &GatewayScheduler,
    capability_cache: &CapabilityCatalogCache,
    upload_store: &UploadStore,
    logger: &JsonLogger,
) -> Result<()> {
    stream
        .set_read_timeout(Some(Duration::from_millis(
            config.gateway_request_timeout_ms(),
        )))
        .context("set gateway read timeout")?;
    stream
        .set_write_timeout(Some(Duration::from_millis(
            config.gateway_request_timeout_ms(),
        )))
        .context("set gateway write timeout")?;
    let peer_addr = stream.peer_addr().ok();

    let context = match parse_http_request(&mut stream, request_counter) {
        Ok(context) => context,
        Err(err) => {
            let request_id = next_request_id(request_counter);
            let response = if err.status_code == 413 {
                json_response(
                    413,
                    json!({
                        "request_id": request_id,
                        "error": err.client_error,
                    }),
                )
            } else {
                json_response(
                    400,
                    json!({
                        "error": "bad_request",
                    }),
                )
            };
            logger.warn(
                "gateway request parse failed",
                json!({
                    "request_id": request_id,
                    "error": format!("{err:#}"),
                }),
            )?;
            return write_http_response(&mut stream, &request_id, response);
        }
    };

    log_request(logger, &context.request_id, &context.request, &remote_addr)?;
    let route = match_route(&context.request.method, &context.request.path);

    if let Err(response) = authorize_request(
        config,
        &context.request,
        peer_addr,
        route_requires_openai_auth_error_shape(route),
    ) {
        log_response(logger, &context.request_id, response.status_code)?;
        return write_http_response(&mut stream, &context.request_id, response);
    }
    let scheduler_status = scheduler.snapshot()?;

    let response = match route {
        Route::Health => health_admin::handle_health_request(
            config,
            core,
            snapshot,
            &scheduler_status,
            &context.request_id,
        )?,
        Route::Ready => {
            health_admin::handle_ready_request(snapshot, &scheduler_status, &context.request_id)
        }
        Route::AdminStatus => health_admin::handle_admin_status_request(
            config,
            core,
            snapshot,
            &scheduler_status,
            &context.request_id,
        )?,
        Route::AdminShutdown => {
            health_admin::handle_admin_shutdown_request(shutdown, &context.request_id)
        }
        Route::AdminClear => health_admin::handle_admin_clear_request(
            core,
            &context.request.body,
            &context.request_id,
        )?,
        Route::Models => {
            metadata::handle_models_request(core, capability_cache, snapshot, &context.request_id)
        }
        Route::Capabilities => {
            metadata::handle_capabilities_request(core, capability_cache, &context.request_id)?
        }
        Route::Provider => {
            metadata::handle_provider_request(core, capability_cache, snapshot, &context.request_id)
        }
        Route::Usage => metadata::handle_usage_request(core, &context.request_id)?,
        Route::Sessions => metadata::handle_sessions_request(core, &context.request_id)?,
        Route::SessionConversations => {
            conversations::handle_session_conversations_request(core, &context)?
        }
        Route::Requests => requests::handle_requests_request(core, &context)?,
        Route::Conversations => {
            conversations::handle_conversations_request(core, &context.request_id)?
        }
        Route::ConversationMessages => {
            conversations::handle_conversation_messages_request(core, &context)?
        }
        Route::ConversationSnapshot => {
            conversations::handle_conversation_snapshot_request(core, &context)?
        }
        Route::ConversationDetail => {
            conversations::handle_conversation_detail_request(core, &context)?
        }
        Route::RequestEvidence => requests::handle_request_evidence_request(core, &context)?,
        Route::RequestSummary => requests::handle_request_summary_request(core, &context)?,
        Route::FilesList => files::handle_files_list_request(upload_store, &context.request_id)?,
        Route::FileUpload => files::handle_file_upload_request(upload_store, &context)?,
        Route::FileDetail => files::handle_file_detail_request(upload_store, &context)?,
        Route::ArtifactsList => {
            artifacts::handle_artifacts_list_request(core, &context.request_id)?
        }
        Route::ArtifactDetail => artifacts::handle_artifact_request(core, &context)?,
        Route::RequestAction => requests::handle_cancel_request(core, scheduler, &context)?,
        Route::ConversationSelect => {
            conversations::handle_conversation_select_request(core, &context)?
        }
        Route::ChatCompletions => {
            return chat::handle_chat_completions_request(
                &mut stream,
                core,
                upload_store,
                scheduler,
                scheduler_status,
                &context,
                logger,
                &remote_addr,
            );
        }
        Route::NotFound => json_response(
            404,
            json!({
                "request_id": context.request_id,
                "error": "not_found",
            }),
        ),
    };

    log_response(logger, &context.request_id, response.status_code)?;
    write_http_response(&mut stream, &context.request_id, response)
}

fn route_requires_openai_auth_error_shape(route: Route) -> bool {
    matches!(
        route,
        Route::Models | Route::Capabilities | Route::ChatCompletions
    )
}
