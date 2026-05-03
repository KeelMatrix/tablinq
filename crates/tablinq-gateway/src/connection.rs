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
    let scheduler_status = scheduler.snapshot()?;

    let response = match match_route(&context.request.method, &context.request.path) {
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
        Route::AdminStatus => match authorize_request(config, &context.request, false) {
            Ok(()) => health_admin::handle_admin_status_request(
                config,
                core,
                snapshot,
                &scheduler_status,
                &context.request_id,
            )?,
            Err(response) => response,
        },
        Route::AdminShutdown => match authorize_request(config, &context.request, false) {
            Ok(()) => health_admin::handle_admin_shutdown_request(shutdown, &context.request_id),
            Err(response) => response,
        },
        Route::AdminClear => match authorize_request(config, &context.request, false) {
            Ok(()) => health_admin::handle_admin_clear_request(
                core,
                &context.request.body,
                &context.request_id,
            )?,
            Err(response) => response,
        },
        Route::Models => match authorize_request(config, &context.request, true) {
            Ok(()) => metadata::handle_models_request(
                core,
                capability_cache,
                snapshot,
                &context.request_id,
            ),
            Err(response) => response,
        },
        Route::Capabilities => match authorize_request(config, &context.request, true) {
            Ok(()) => {
                metadata::handle_capabilities_request(core, capability_cache, &context.request_id)?
            }
            Err(response) => response,
        },
        Route::Provider => match authorize_request(config, &context.request, false) {
            Ok(()) => metadata::handle_provider_request(
                core,
                capability_cache,
                snapshot,
                &context.request_id,
            ),
            Err(response) => response,
        },
        Route::Usage => match authorize_request(config, &context.request, false) {
            Ok(()) => metadata::handle_usage_request(core, &context.request_id)?,
            Err(response) => response,
        },
        Route::Sessions => match authorize_request(config, &context.request, false) {
            Ok(()) => metadata::handle_sessions_request(core, &context.request_id)?,
            Err(response) => response,
        },
        Route::SessionConversations => match authorize_request(config, &context.request, false) {
            Ok(()) => conversations::handle_session_conversations_request(core, &context)?,
            Err(response) => response,
        },
        Route::Requests => match authorize_request(config, &context.request, false) {
            Ok(()) => requests::handle_requests_request(core, &context)?,
            Err(response) => response,
        },
        Route::Conversations => match authorize_request(config, &context.request, false) {
            Ok(()) => conversations::handle_conversations_request(core, &context.request_id)?,
            Err(response) => response,
        },
        Route::ConversationMessages => match authorize_request(config, &context.request, false) {
            Ok(()) => conversations::handle_conversation_messages_request(core, &context)?,
            Err(response) => response,
        },
        Route::ConversationSnapshot => match authorize_request(config, &context.request, false) {
            Ok(()) => conversations::handle_conversation_snapshot_request(core, &context)?,
            Err(response) => response,
        },
        Route::ConversationDetail => match authorize_request(config, &context.request, false) {
            Ok(()) => conversations::handle_conversation_detail_request(core, &context)?,
            Err(response) => response,
        },
        Route::RequestEvidence => match authorize_request(config, &context.request, false) {
            Ok(()) => requests::handle_request_evidence_request(core, &context)?,
            Err(response) => response,
        },
        Route::RequestSummary => match authorize_request(config, &context.request, false) {
            Ok(()) => requests::handle_request_summary_request(core, &context)?,
            Err(response) => response,
        },
        Route::FilesList => match authorize_request(config, &context.request, false) {
            Ok(()) => files::handle_files_list_request(upload_store, &context.request_id)?,
            Err(response) => response,
        },
        Route::FileUpload => match authorize_request(config, &context.request, false) {
            Ok(()) => files::handle_file_upload_request(upload_store, &context)?,
            Err(response) => response,
        },
        Route::FileDetail => match authorize_request(config, &context.request, false) {
            Ok(()) => files::handle_file_detail_request(upload_store, &context)?,
            Err(response) => response,
        },
        Route::ArtifactsList => match authorize_request(config, &context.request, false) {
            Ok(()) => artifacts::handle_artifacts_list_request(core, &context.request_id)?,
            Err(response) => response,
        },
        Route::ArtifactDetail => match authorize_request(config, &context.request, false) {
            Ok(()) => artifacts::handle_artifact_request(core, &context)?,
            Err(response) => response,
        },
        Route::RequestAction => match authorize_request(config, &context.request, false) {
            Ok(()) => requests::handle_cancel_request(core, scheduler, &context)?,
            Err(response) => response,
        },
        Route::ConversationSelect => match authorize_request(config, &context.request, false) {
            Ok(()) => conversations::handle_conversation_select_request(core, &context)?,
            Err(response) => response,
        },
        Route::ChatCompletions => {
            if let Err(response) = authorize_request(config, &context.request, true) {
                response
            } else {
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
