use anyhow::{Context, Result};
use chrono::Utc;
use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

pub struct JsonLogger {
    file: Mutex<std::fs::File>,
}

impl JsonLogger {
    pub fn new(path: &Path) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("open {}", path.display()))?;
        Ok(Self {
            file: Mutex::new(file),
        })
    }

    pub fn info(&self, message: &str, fields: Value) -> Result<()> {
        self.write("info", message, fields)
    }

    pub fn warn(&self, message: &str, fields: Value) -> Result<()> {
        self.write("warn", message, fields)
    }

    fn write(&self, level: &str, message: &str, fields: Value) -> Result<()> {
        let line = json!({
            "ts": Utc::now().to_rfc3339(),
            "level": level,
            "message": message,
            "fields": fields,
        });

        let mut file = self.file.lock().expect("logger mutex poisoned");
        serde_json::to_writer(&mut *file, &line).context("write log line")?;
        file.write_all(b"\n").context("write log newline")?;
        file.flush().context("flush log line")?;
        Ok(())
    }
}
