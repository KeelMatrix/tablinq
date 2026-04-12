use crate::ipc::daemon_is_responsive;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use shared_protocol::StatePaths;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const STALE_AFTER_SECS: i64 = 5;

#[derive(Debug)]
pub struct LockGuard {
    path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct LockMetadata {
    pid: u32,
    created_at: DateTime<Utc>,
    ipc_endpoint: String,
}

pub fn acquire(paths: &StatePaths) -> Result<LockGuard> {
    if paths.lock_path.exists() {
        handle_existing_lock(&paths.lock_path, &paths.ipc_endpoint)?;
    }

    let metadata = LockMetadata {
        pid: std::process::id(),
        created_at: Utc::now(),
        ipc_endpoint: paths.ipc_endpoint.clone(),
    };
    let raw = serde_json::to_string_pretty(&metadata).context("encode lock metadata")?;

    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&paths.lock_path)
        .with_context(|| format!("create {}", paths.lock_path.display()))?;
    file.write_all(raw.as_bytes())
        .with_context(|| format!("write {}", paths.lock_path.display()))?;

    Ok(LockGuard {
        path: paths.lock_path.clone(),
    })
}

fn handle_existing_lock(path: &Path, ipc_endpoint: &str) -> Result<()> {
    if daemon_is_responsive(ipc_endpoint) {
        bail!("daemon already running");
    }

    let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let metadata: LockMetadata = serde_json::from_str(&raw).context("decode lock metadata")?;
    let age = Utc::now() - metadata.created_at;

    if age > Duration::seconds(STALE_AFTER_SECS) {
        fs::remove_file(path).with_context(|| format!("remove stale {}", path.display()))?;
        return Ok(());
    }

    bail!("daemon startup lock exists but no responsive daemon was found")
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
