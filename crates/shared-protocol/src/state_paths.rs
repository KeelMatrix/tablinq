use crate::{APP_NAME, CHROME_ATTACH_PROFILE_DIR_NAME};
use std::env;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatePaths {
    pub state_root: PathBuf,
    pub config_dir: PathBuf,
    pub config_path: PathBuf,
    pub logs_dir: PathBuf,
    pub log_path: PathBuf,
    pub db_dir: PathBuf,
    pub db_path: PathBuf,
    pub browser_profile_dir: PathBuf,
    pub chrome_attach_profile_dir: PathBuf,
    pub downloads_dir: PathBuf,
    pub crash_bundles_dir: PathBuf,
    pub lock_path: PathBuf,
    pub ipc_endpoint: String,
}

pub fn resolve_state_root(explicit: Option<&Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path.to_path_buf();
    }

    if let Ok(value) = env::var("TABLINQ_STATE_ROOT") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    default_state_root()
}

pub fn default_state_root() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(value) = env::var("LOCALAPPDATA") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed).join(APP_NAME);
            }
        }

        if let Ok(value) = env::var("USERPROFILE") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed)
                    .join("AppData")
                    .join("Local")
                    .join(APP_NAME);
            }
        }
    }

    #[cfg(not(windows))]
    {
        if let Ok(value) = env::var("XDG_STATE_HOME") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed).join(APP_NAME.to_lowercase());
            }
        }

        if let Ok(value) = env::var("HOME") {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed)
                    .join(".local")
                    .join("state")
                    .join(APP_NAME.to_lowercase());
            }
        }
    }

    PathBuf::from(".tablinq")
}

pub fn build_state_paths(state_root: impl AsRef<Path>) -> StatePaths {
    let state_root = state_root.as_ref().to_path_buf();
    let config_dir = state_root.join("config");
    let logs_dir = state_root.join("logs");
    let db_dir = state_root.join("db");
    let browser_profile_dir = state_root.join("browser-profile");
    let chrome_attach_profile_dir = state_root.join(CHROME_ATTACH_PROFILE_DIR_NAME);
    let downloads_dir = state_root.join("downloads");
    let crash_bundles_dir = state_root.join("crash-bundles");

    StatePaths {
        config_path: config_dir.join("config.toml"),
        log_path: logs_dir.join("gptd.log"),
        db_path: db_dir.join("tablinq.sqlite3"),
        lock_path: state_root.join("daemon.lock"),
        ipc_endpoint: ipc_endpoint_for_root(&state_root),
        state_root,
        config_dir,
        logs_dir,
        db_dir,
        browser_profile_dir,
        chrome_attach_profile_dir,
        downloads_dir,
        crash_bundles_dir,
    }
}

pub fn ipc_endpoint_for_root(state_root: impl AsRef<Path>) -> String {
    let input = state_root.as_ref().to_string_lossy();
    format!("tablinq-{:016x}", stable_hash(input.as_bytes()))
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
