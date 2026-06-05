use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use gptd::AppConfig;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) const MAX_STAGED_UPLOAD_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct StagedFileMetadata {
    pub(crate) file_id: String,
    pub(crate) file_name: String,
    pub(crate) content_type: String,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
    pub(crate) created_at: DateTime<Utc>,
    pub(crate) expires_at: DateTime<Utc>,
    pub(crate) staged_path: String,
    #[serde(default)]
    pub(crate) linked_request_ids: Vec<String>,
    #[serde(default)]
    pub(crate) linked_conversation_ids: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct UploadStore {
    root: PathBuf,
    retention_secs: i64,
    next_id: AtomicU64,
}

impl UploadStore {
    pub(crate) fn load(config: &AppConfig) -> Result<Self> {
        let root = gateway_uploads_dir(config);
        fs::create_dir_all(&root).with_context(|| format!("create {}", root.display()))?;
        let store = Self {
            root,
            retention_secs: crate::STAGED_UPLOAD_RETENTION_SECS,
            next_id: AtomicU64::new(0),
        };
        store.cleanup_expired()?;
        Ok(store)
    }

    pub(crate) fn stage_upload(
        &self,
        file_name: &str,
        content_type: Option<&str>,
        body: &[u8],
    ) -> Result<StagedFileMetadata> {
        self.cleanup_expired()?;
        if body.len() > MAX_STAGED_UPLOAD_BYTES {
            anyhow::bail!("upload exceeds 50 MiB limit");
        }

        let safe_name = sanitize_upload_file_name(file_name);
        let created_at = Utc::now();
        let file_id = format!(
            "file-{}-{}",
            created_at.timestamp_millis(),
            self.next_id.fetch_add(1, Ordering::SeqCst)
        );
        let dir = self.root.join(&file_id);
        fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;

        let staged_path = dir.join(&safe_name);
        fs::write(&staged_path, body)
            .with_context(|| format!("write {}", staged_path.display()))?;

        let metadata = StagedFileMetadata {
            file_id: file_id.clone(),
            file_name: safe_name,
            content_type: normalize_content_type(content_type),
            size_bytes: body.len() as u64,
            sha256: sha256_hex(body),
            created_at,
            expires_at: created_at + chrono::Duration::seconds(self.retention_secs),
            staged_path: staged_path.display().to_string(),
            linked_request_ids: vec![],
            linked_conversation_ids: vec![],
        };
        self.write_metadata(&dir, &metadata)?;
        Ok(metadata)
    }

    pub(crate) fn resolve_file_inputs(
        &self,
        file_paths: Vec<String>,
        file_ids: &[String],
    ) -> Result<Vec<String>> {
        self.cleanup_expired()?;
        let mut resolved = file_paths;
        for file_id in file_ids {
            let metadata = self
                .get(file_id)?
                .with_context(|| format!("staged file {file_id} was not found"))?;
            resolved.push(metadata.staged_path);
        }
        Ok(resolved)
    }

    pub(crate) fn get(&self, file_id: &str) -> Result<Option<StagedFileMetadata>> {
        self.cleanup_expired()?;
        let dir = self.root.join(file_id);
        if !dir.exists() {
            return Ok(None);
        }
        let metadata = read_staged_file_metadata(&dir)?;
        if !Path::new(&metadata.staged_path).exists() {
            let _ = fs::remove_dir_all(&dir);
            return Ok(None);
        }
        Ok(Some(metadata))
    }

    pub(crate) fn list(&self) -> Result<Vec<StagedFileMetadata>> {
        self.cleanup_expired()?;
        let mut items = Vec::new();
        for entry in
            fs::read_dir(&self.root).with_context(|| format!("read {}", self.root.display()))?
        {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            if let Ok(metadata) = read_staged_file_metadata(&entry.path())
                && Path::new(&metadata.staged_path).exists()
            {
                items.push(metadata);
            }
        }
        items.sort_by_key(|item| std::cmp::Reverse(item.created_at));
        Ok(items)
    }

    pub(crate) fn mark_linked_usage(
        &self,
        file_ids: &[String],
        request_id: &str,
        conversation_id: Option<&str>,
    ) -> Result<()> {
        if file_ids.is_empty() {
            return Ok(());
        }

        self.cleanup_expired()?;
        for file_id in file_ids {
            let dir = self.root.join(file_id);
            if !dir.exists() {
                continue;
            }
            let mut metadata = match read_staged_file_metadata(&dir) {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            remember_link(&mut metadata.linked_request_ids, request_id);
            if let Some(conversation_id) = conversation_id {
                remember_link(&mut metadata.linked_conversation_ids, conversation_id);
            }
            self.write_metadata(&dir, &metadata)?;
        }
        Ok(())
    }

    pub(crate) fn cleanup_expired(&self) -> Result<()> {
        let now = Utc::now();
        for entry in
            fs::read_dir(&self.root).with_context(|| format!("read {}", self.root.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_dir() {
                let _ = fs::remove_file(path);
                continue;
            }

            let expired_or_invalid = match read_staged_file_metadata(&path) {
                Ok(metadata) => {
                    metadata.expires_at <= now || !Path::new(&metadata.staged_path).exists()
                }
                Err(_) => true,
            };
            if expired_or_invalid {
                let _ = fs::remove_dir_all(&path);
            }
        }
        Ok(())
    }

    fn write_metadata(&self, dir: &Path, metadata: &StagedFileMetadata) -> Result<()> {
        let path = staged_metadata_path(dir);
        let raw = serde_json::to_vec_pretty(metadata).context("encode staged upload metadata")?;
        fs::write(&path, raw).with_context(|| format!("write {}", path.display()))
    }
}

fn read_staged_file_metadata(dir: &Path) -> Result<StagedFileMetadata> {
    let path = staged_metadata_path(dir);
    let raw = fs::read(&path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_slice(&raw).context("decode staged upload metadata")
}

pub(crate) fn staged_metadata_path(dir: &Path) -> PathBuf {
    dir.join("metadata.json")
}

pub(crate) fn gateway_uploads_dir(config: &AppConfig) -> PathBuf {
    config.paths.state_root.join("gateway-uploads")
}

fn sanitize_upload_file_name(file_name: &str) -> String {
    let candidate = file_name
        .rsplit(['/', '\\'])
        .next()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("upload.bin");
    let mut sanitized = candidate
        .chars()
        .map(|ch| {
            if matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || ch.is_control()
            {
                '_'
            } else {
                ch
            }
        })
        .collect::<String>()
        .trim()
        .trim_end_matches(['.', ' '])
        .to_owned();
    if sanitized.is_empty() {
        sanitized = "upload.bin".to_owned();
    }
    sanitized
}

fn remember_link(items: &mut Vec<String>, value: &str) {
    if value.trim().is_empty() || items.iter().any(|item| item == value) {
        return;
    }
    items.push(value.to_owned());
    items.sort();
}

fn normalize_content_type(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("application/octet-stream")
        .to_owned()
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut text = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut text, "{byte:02x}");
    }
    text
}
