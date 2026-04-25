mod auth;
mod browser;
pub mod build_info;
mod capabilities;
mod chat;
mod connection;
mod errors;
mod handlers;
mod http;
mod json;
mod openai;
mod process_control;
mod routes;
mod scheduler;
mod server;
mod sse;
mod uploads;

use anyhow::Result;
use gptd::AppConfig;
use std::time::Duration;

pub use crate::process_control::{
    GatewayStatusSnapshot, gateway_doctor, gateway_status, load_status_snapshot,
    start_gateway_process, stop_gateway_process,
};

pub(crate) const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const STARTUP_PROBE_TIMEOUT_MS: u64 = 1_000;
pub(crate) const STAGED_UPLOAD_RETENTION_SECS: i64 = 24 * 60 * 60;

pub(crate) use crate::http::http_request;
pub(crate) use crate::routes::{ADMIN_SHUTDOWN_PATH, ADMIN_STATUS_PATH, HEALTH_PATH, READY_PATH};

pub fn run_gateway(config: AppConfig) -> Result<()> {
    server::run_gateway(config)
}

#[cfg(test)]
use crate::capabilities::CapabilityCatalogCache;
#[cfg(test)]
pub(crate) use crate::errors::map_openai_error;
#[cfg(test)]
pub(crate) use crate::http::{
    MAX_HTTP_HEADER_LINE_BYTES, MAX_HTTP_REQUEST_LINE_BYTES, MAX_NON_UPLOAD_REQUEST_BODY_BYTES,
    SimpleHttpResponse, http_request_with_declared_content_length,
};
#[cfg(test)]
use crate::process_control::gateway_log_path;
#[cfg(test)]
pub(crate) use crate::routes::{
    V1_ARTIFACTS_PATH, V1_CAPABILITIES_PATH, V1_CHAT_COMPLETIONS_PATH, V1_CONVERSATIONS_PATH,
    V1_CONVERSATIONS_SELECT_PATH, V1_FILES_PATH, V1_MODELS_PATH, V1_PROVIDER_PATH,
    V1_SESSIONS_PATH, V1_USAGE_PATH,
};
#[cfg(test)]
use crate::uploads::{StagedFileMetadata, UploadStore, gateway_uploads_dir, staged_metadata_path};
#[cfg(test)]
use chrono::Utc;
#[cfg(test)]
use serde_json::{Value, json};
#[cfg(test)]
use shared_protocol::{CapabilityReport, JobStatus};
#[cfg(test)]
use std::collections::BTreeMap;
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::io::{BufRead, BufReader, Read, Write};
#[cfg(test)]
use std::net::TcpStream;
#[cfg(test)]
use std::process::Command;
#[cfg(test)]
use std::thread;
#[cfg(test)]
use std::time::Instant;

#[cfg(test)]
mod tests;
