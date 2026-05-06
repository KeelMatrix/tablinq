use crate::GatewayStatusSnapshot;
use crate::capabilities::CapabilityCatalogCache;
use crate::connection::handle_connection;
use crate::process_control::{
    GatewayRuntimeGuard, acquire_gateway_lock, gateway_log_path, gateway_status_path,
    write_status_snapshot,
};
use crate::scheduler::GatewayScheduler;
use crate::uploads::UploadStore;
use anyhow::{Context, Result};
use chrono::Utc;
use gptd::logging::JsonLogger;
use gptd::{AppConfig, CoreService};
use serde_json::json;
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

pub(crate) fn run_gateway(config: AppConfig) -> Result<()> {
    let log_path = gateway_log_path(&config);
    let logger = Arc::new(JsonLogger::new(&log_path)?);
    config.require_gateway_api_key()?;
    let core = Arc::new(CoreService::new(config.clone())?);
    let capability_cache = Arc::new(CapabilityCatalogCache::load(&config)?);
    let upload_store = Arc::new(UploadStore::load(&config)?);
    let _lock = acquire_gateway_lock(&config)?;

    let listener =
        TcpListener::bind((config.gateway_host(), config.gateway_port())).with_context(|| {
            format!(
                "bind gateway {}:{}",
                config.gateway_host(),
                config.gateway_port()
            )
        })?;
    let actual_port = listener
        .local_addr()
        .context("resolve gateway local address")?
        .port();
    listener
        .set_nonblocking(true)
        .context("set gateway listener nonblocking")?;

    if config.warm_mode()
        && let Err(err) = core.warm_browser_if_enabled()
    {
        logger.warn(
            "gateway warm browser open failed",
            json!({
                "error": format!("{err:#}"),
            }),
        )?;
    }

    let snapshot = Arc::new(GatewayStatusSnapshot {
        pid: std::process::id(),
        started_at: Utc::now(),
        state_root: config.paths.state_root.display().to_string(),
        host: config.gateway_host().to_owned(),
        port: actual_port,
        api_key_present: config.gateway_api_key().is_some(),
        max_concurrent_upstream_jobs: config.gateway_max_concurrent_upstream_jobs(),
        queue_length: config.gateway_queue_length(),
        queue_timeout_ms: config.gateway_queue_timeout_ms(),
        per_client_limit: config.gateway_per_client_limit(),
        request_timeout_ms: config.gateway_request_timeout_ms(),
        ask_timeout_ms: config.ask_timeout_ms(),
        extended_thinking_timeout_ms: config.extended_thinking_timeout_ms(),
        research_timeout_ms: config.research_timeout_ms(),
        safe_mode_behavior: config.gateway_safe_mode_behavior().to_owned(),
        log_path: log_path.display().to_string(),
    });
    write_status_snapshot(&config, &snapshot)?;
    let _runtime_guard = GatewayRuntimeGuard {
        status_path: gateway_status_path(&config),
    };

    logger.info(
        "gateway ready",
        json!({
            "request_id": "bootstrap",
            "host": snapshot.host,
            "port": snapshot.port,
            "safe_mode_behavior": snapshot.safe_mode_behavior,
            "max_concurrent_upstream_jobs": snapshot.max_concurrent_upstream_jobs,
            "queue_length": snapshot.queue_length,
            "queue_timeout_ms": snapshot.queue_timeout_ms,
            "per_client_limit": snapshot.per_client_limit,
        }),
    )?;

    let shutdown = Arc::new(AtomicBool::new(false));
    let request_counter = Arc::new(AtomicU64::new(0));
    let scheduler = Arc::new(GatewayScheduler::new(&config));

    while !shutdown.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, addr)) => {
                let logger = Arc::clone(&logger);
                let core = Arc::clone(&core);
                let snapshot = Arc::clone(&snapshot);
                let shutdown = Arc::clone(&shutdown);
                let request_counter = Arc::clone(&request_counter);
                let scheduler = Arc::clone(&scheduler);
                let capability_cache = Arc::clone(&capability_cache);
                let upload_store = Arc::clone(&upload_store);
                let config = config.clone();
                thread::spawn(move || {
                    if let Err(err) = handle_connection(
                        stream,
                        addr.to_string(),
                        &config,
                        &core,
                        &snapshot,
                        &shutdown,
                        &request_counter,
                        &scheduler,
                        &capability_cache,
                        &upload_store,
                        &logger,
                    ) {
                        let _ = logger.warn(
                            "gateway connection failed",
                            json!({
                                "error": format!("{err:#}"),
                            }),
                        );
                    }
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(err) => return Err(err).context("accept gateway connection"),
        }
    }
    Ok(())
}
