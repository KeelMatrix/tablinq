use crate::BrowserConnectionMode;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BuildMetadata {
    pub application: String,
    pub version: String,
    pub git_commit: Option<String>,
    pub git_dirty: bool,
    pub target: String,
    pub profile: String,
    pub built_at_unix_epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfigSnapshot {
    pub host: String,
    pub port: u16,
    pub api_key_present: bool,
    pub max_concurrent_upstream_jobs: u32,
    pub queue_length: u32,
    pub queue_timeout_ms: u64,
    pub per_client_limit: u32,
    pub request_timeout_ms: u64,
    pub ask_timeout_ms: u64,
    pub extended_thinking_timeout_ms: u64,
    pub research_timeout_ms: u64,
    pub safe_mode_behavior: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConfigSnapshot {
    pub version: u32,
    pub state_root: String,
    pub config_dir: String,
    pub config_path: String,
    pub logs_dir: String,
    pub log_path: String,
    pub db_dir: String,
    pub db_path: String,
    pub browser_profile_dir: String,
    pub chrome_attach_profile_dir: String,
    pub downloads_dir: String,
    pub crash_bundles_dir: String,
    pub ipc_endpoint: String,
    pub warm_mode: bool,
    pub safe_mode: bool,
    pub browser_target_url: String,
    pub browser_path: Option<String>,
    pub browser_connection_mode: BrowserConnectionMode,
    pub browser_debug_endpoint: String,
    pub browser_debug_host: String,
    pub browser_debug_port: u16,
    pub browser_channel: String,
    pub browser_extension_required: bool,
    pub browser_auto_open_target_tab: bool,
    pub gateway: GatewayConfigSnapshot,
}
