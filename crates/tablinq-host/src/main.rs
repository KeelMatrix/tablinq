use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde_json::{Value, json};
use shared_protocol::{BuildMetadata, build_state_paths, resolve_state_root};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};

fn main() -> Result<()> {
    let request = read_native_message(std::io::stdin().lock()).context("read native message")?;
    persist_message(&request)?;

    let response = json!({
        "ok": true,
        "host": "com.tablinq.bridge",
        "received_type": request.get("type").and_then(Value::as_str).unwrap_or("unknown"),
        "received_at": Utc::now().to_rfc3339(),
        "build": build_metadata(),
    });
    write_native_message(std::io::stdout().lock(), &response).context("write native response")
}

fn build_metadata() -> BuildMetadata {
    BuildMetadata {
        application: "tablinq-host".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        git_commit: env_commit(),
        git_dirty: env!("TABLINQ_BUILD_DIRTY") == "true",
        target: env!("TABLINQ_BUILD_TARGET").to_owned(),
        profile: env!("TABLINQ_BUILD_PROFILE").to_owned(),
        built_at_unix_epoch: env!("TABLINQ_BUILD_UNIX_EPOCH").parse().unwrap_or(0),
    }
}

fn env_commit() -> Option<String> {
    let value = option_env!("TABLINQ_BUILD_COMMIT")?;
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "unknown" {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn persist_message(message: &Value) -> Result<()> {
    let paths = build_state_paths(resolve_state_root(None));
    persist_message_to_path(&paths.logs_dir, message)
}

fn persist_message_to_path(logs_dir: &std::path::Path, message: &Value) -> Result<()> {
    fs::create_dir_all(logs_dir).with_context(|| format!("create {}", logs_dir.display()))?;
    let log_path = logs_dir.join("native-host.log");
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("open {}", log_path.display()))?;
    writeln!(
        file,
        "{}",
        serde_json::to_string(&json!({
            "received_at": Utc::now().to_rfc3339(),
            "payload": message,
        }))
        .context("serialize native-host log line")?
    )
    .with_context(|| format!("write {}", log_path.display()))
}

fn read_native_message(mut reader: impl Read) -> Result<Value> {
    let mut header = [0u8; 4];
    reader
        .read_exact(&mut header)
        .context("read native header")?;
    let len = u32::from_le_bytes(header) as usize;
    if len == 0 {
        bail!("native message length must not be zero");
    }
    let mut payload = vec![0u8; len];
    reader
        .read_exact(&mut payload)
        .context("read native payload")?;
    serde_json::from_slice(&payload).context("decode native payload")
}

fn write_native_message(mut writer: impl Write, message: &Value) -> Result<()> {
    let payload = serde_json::to_vec(message).context("encode native payload")?;
    let len = payload
        .len()
        .try_into()
        .context("native payload is too large")?;
    writer
        .write_all(&u32::to_le_bytes(len))
        .context("write native header")?;
    writer.write_all(&payload).context("write native payload")?;
    writer.flush().context("flush native payload")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn native_message_round_trips() {
        let value = json!({
            "type": "capability_report",
            "report": {
                "reported_at": "2026-04-13T00:00:00Z"
            }
        });
        let mut output = Vec::new();
        write_native_message(&mut output, &value).unwrap();

        let decoded = read_native_message(output.as_slice()).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn native_message_rejects_zero_length_payload() {
        let err = read_native_message([0, 0, 0, 0].as_slice()).unwrap_err();
        assert!(err.to_string().contains("must not be zero"));
    }

    #[test]
    fn persist_message_writes_log_file() {
        let dir = tempdir().unwrap();
        let logs_dir = build_state_paths(dir.path()).logs_dir;
        persist_message_to_path(&logs_dir, &json!({ "type": "capability_report" })).unwrap();
        let log_path = logs_dir.join("native-host.log");
        let raw = fs::read_to_string(log_path).unwrap();
        assert!(raw.contains("\"capability_report\""));
    }
}
