use crate::auth::authorize_request;
use crate::capabilities::CapabilityCatalogCache;
use crate::errors::is_client_disconnect_error;
use crate::handlers::{
    artifacts, browser, chat, conversations, files, health_admin, metadata, requests,
};
use crate::http::{
    HttpResponse, json_response, log_request, log_response, next_request_id, parse_http_request,
    write_http_response,
};
use crate::json::api_error_response;
use crate::process_control::GatewayStatusSnapshot;
use crate::routes::{Route, match_route, method_not_allowed_for_path, normalized_request_path};
use crate::scheduler::GatewayScheduler;
use crate::uploads::UploadStore;
use anyhow::{Context, Result};
use gptd::logging::JsonLogger;
use gptd::{AppConfig, CoreService};
use serde_json::json;
use std::any::Any;
use std::net::TcpStream;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::time::Duration;

enum DispatchResult {
    Response(HttpResponse),
    Written,
}

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
        .set_nonblocking(false)
        .context("set gateway connection blocking")?;
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
                api_error_response(
                    413,
                    &request_id,
                    "invalid_request_error",
                    err.code,
                    format!("{err:#}"),
                )
            } else {
                api_error_response(
                    400,
                    &request_id,
                    "invalid_request_error",
                    err.code,
                    format!("{err:#}"),
                )
            };
            let _ = logger.warn(
                "gateway request parse failed",
                json!({
                    "request_id": request_id,
                    "error": format!("{err:#}"),
                }),
            );
            return write_http_response_logged(&mut stream, logger, &request_id, None, response);
        }
    };

    let _ = log_request(logger, &context.request_id, &context.request, &remote_addr);
    let route = match_route(&context.request.method, &context.request.path);

    if let Err(response) = authorize_request(
        config,
        &context.request,
        peer_addr,
        &context.request_id,
        route_requires_openai_auth_error_shape(&context.request.path),
    ) {
        let _ = log_response(logger, &context.request_id, response.status_code);
        return write_http_response_logged(
            &mut stream,
            logger,
            &context.request_id,
            Some(route),
            response,
        );
    }
    let scheduler_status = match scheduler.snapshot() {
        Ok(status) => status,
        Err(err) => {
            let response = internal_server_error_response(&context.request_id, &err);
            let _ = log_response(logger, &context.request_id, response.status_code);
            return write_http_response_logged(
                &mut stream,
                logger,
                &context.request_id,
                Some(route),
                response,
            );
        }
    };

    let response_result = catch_unwind(AssertUnwindSafe(|| -> Result<DispatchResult> {
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
            Route::Models => metadata::handle_models_request(
                core,
                capability_cache,
                snapshot,
                &context.request_id,
            ),
            Route::Capabilities => {
                metadata::handle_capabilities_request(core, capability_cache, &context.request_id)?
            }
            Route::Provider => metadata::handle_provider_request(
                core,
                capability_cache,
                snapshot,
                &context.request_id,
            ),
            Route::Status => health_admin::handle_status_request(
                core,
                snapshot,
                &scheduler_status,
                &context.request_id,
            ),
            Route::Doctor => health_admin::handle_doctor_request(
                config,
                core,
                &scheduler_status,
                &context.request_id,
            )?,
            Route::Config => health_admin::handle_config_request(core, &context.request_id),
            Route::Jobs => requests::handle_jobs_request(core, &context)?,
            Route::JobDetail => requests::handle_job_detail_request(core, &context)?,
            Route::JobCancel => requests::handle_job_cancel_request(core, &context)?,
            Route::Usage => metadata::handle_usage_request(core, &context)?,
            Route::BrowserAttach => browser::handle_browser_attach_request(
                core,
                &context.request.body,
                &context.request_id,
            )?,
            Route::BrowserDetach => browser::handle_browser_detach_request(
                core,
                &context.request.body,
                &context.request_id,
            )?,
            Route::BrowserProfileInfo => {
                browser::handle_browser_profile_info_request(config, core, &context.request_id)?
            }
            Route::BrowserConnectionInfo => {
                browser::handle_browser_connection_info_request(config, core, &context.request_id)?
            }
            Route::Sessions => metadata::handle_sessions_request(core, &context.request_id)?,
            Route::SessionConversations => {
                conversations::handle_session_conversations_request(core, &context)?
            }
            Route::Requests => requests::handle_requests_request(core, &context)?,
            Route::Conversations => conversations::handle_conversations_request(core, &context)?,
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
            Route::FilesList => files::handle_files_list_request(upload_store, &context)?,
            Route::FileUpload => files::handle_file_upload_request(upload_store, &context)?,
            Route::FileDetail => files::handle_file_detail_request(upload_store, &context)?,
            Route::ArtifactsList => artifacts::handle_artifacts_list_request(core, &context)?,
            Route::ArtifactDetail => artifacts::handle_artifact_request(core, &context)?,
            Route::RequestAction => requests::handle_cancel_request(core, scheduler, &context)?,
            Route::ConversationSelect => {
                conversations::handle_conversation_select_request(core, &context)?
            }
            Route::ChatCompletions => {
                return match catch_unwind(AssertUnwindSafe(|| {
                    chat::handle_chat_completions_request(
                        &mut stream,
                        core,
                        upload_store,
                        scheduler,
                        scheduler_status,
                        &context,
                        logger,
                        &remote_addr,
                    )
                })) {
                    Ok(Ok(chat::ChatCompletionsResult::Response(response))) => {
                        Ok(DispatchResult::Response(response))
                    }
                    Ok(Ok(chat::ChatCompletionsResult::Written)) => Ok(DispatchResult::Written),
                    Ok(Err(err)) if is_client_disconnect_error(&err) => Err(err),
                    Ok(Err(err)) => Ok(DispatchResult::Response(internal_server_error_response(
                        &context.request_id,
                        &err,
                    ))),
                    Err(panic) => Ok(DispatchResult::Response(panic_response(
                        &context.request_id,
                        panic,
                    ))),
                };
            }
            Route::NotFound => {
                if let Some(allowed_method) = method_not_allowed_for_path(&context.request.path) {
                    let mut response =
                        if normalized_request_path(&context.request.path).starts_with("/v1/") {
                            api_error_response(
                                405,
                                &context.request_id,
                                "invalid_request_error",
                                "method_not_allowed",
                                format!(
                                    "{} is not allowed for {}; use {}",
                                    context.request.method,
                                    normalized_request_path(&context.request.path),
                                    allowed_method
                                ),
                            )
                        } else {
                            json_response(
                                405,
                                json!({
                                    "request_id": context.request_id,
                                    "error": "method_not_allowed",
                                }),
                            )
                        };
                    response
                        .headers
                        .push(("allow".to_owned(), allowed_method.to_owned()));
                    response
                } else if normalized_request_path(&context.request.path).starts_with("/v1/") {
                    api_error_response(
                        404,
                        &context.request_id,
                        "not_found_error",
                        "not_found",
                        format!(
                            "{} was not found",
                            normalized_request_path(&context.request.path)
                        ),
                    )
                } else {
                    json_response(
                        404,
                        json!({
                            "request_id": context.request_id,
                            "error": "not_found",
                        }),
                    )
                }
            }
        };
        Ok(DispatchResult::Response(response))
    }));

    let response = match response_result {
        Ok(Ok(DispatchResult::Response(response))) => response,
        Ok(Ok(DispatchResult::Written)) => return Ok(()),
        Ok(Err(err)) if is_client_disconnect_error(&err) => return Err(err),
        Ok(Err(err)) => internal_server_error_response(&context.request_id, &err),
        Err(panic) => panic_response(&context.request_id, panic),
    };

    let _ = log_response(logger, &context.request_id, response.status_code);
    write_http_response_logged(
        &mut stream,
        logger,
        &context.request_id,
        Some(route),
        response,
    )
}

fn internal_server_error_response(request_id: &str, err: &anyhow::Error) -> HttpResponse {
    api_error_response(
        500,
        request_id,
        "server_error",
        "internal_error",
        format!("{request_id}: {err:#}"),
    )
}

fn panic_response(request_id: &str, panic: Box<dyn Any + Send>) -> HttpResponse {
    api_error_response(
        500,
        request_id,
        "server_error",
        "handler_panic",
        format!(
            "{request_id}: request handler panicked: {}",
            panic_message(&panic)
        ),
    )
}

fn panic_message(panic: &Box<dyn Any + Send>) -> &str {
    if let Some(message) = panic.downcast_ref::<&str>() {
        message
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.as_str()
    } else {
        "unknown panic payload"
    }
}

fn route_requires_openai_auth_error_shape(path: &str) -> bool {
    normalized_request_path(path).starts_with("/v1/")
}

fn write_http_response_logged(
    stream: &mut TcpStream,
    logger: &JsonLogger,
    request_id: &str,
    route: Option<Route>,
    response: HttpResponse,
) -> Result<()> {
    let status_code = response.status_code;
    let route = route
        .map(|route| format!("{route:?}"))
        .unwrap_or_else(|| "RequestParse".to_owned());
    match write_http_response(stream, request_id, response) {
        Ok(()) => Ok(()),
        Err(err) if is_client_disconnect_error(&err) => {
            let _ = logger.warn(
                "gateway client closed before response completed",
                json!({
                    "request_id": request_id,
                    "route": route,
                    "status_code": status_code,
                    "phase": "write_response",
                    "error": format!("{err:#}"),
                }),
            );
            Err(err)
        }
        Err(err) => {
            let _ = logger.warn(
                "gateway response write failed",
                json!({
                    "request_id": request_id,
                    "route": route,
                    "status_code": status_code,
                    "phase": "write_response",
                    "error": format!("{err:#}"),
                }),
            );
            Err(err)
        }
    }
}
