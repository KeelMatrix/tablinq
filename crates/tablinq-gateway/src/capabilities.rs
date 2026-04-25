use crate::browser::discover_capabilities;
use anyhow::{Context, Result};
use chrono::Utc;
use gptd::{AppConfig, CoreService};
use serde::{Deserialize, Serialize};
use shared_protocol::{BrowserStatusSnapshot, CapabilityReport};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct CapabilityCacheEntry {
    pub(crate) cache_key: String,
    pub(crate) profile_dir: String,
    pub(crate) workspace_key: String,
    pub(crate) cached_at: chrono::DateTime<Utc>,
    pub(crate) report: CapabilityReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct CapabilityCacheState {
    entries: BTreeMap<String, CapabilityCacheEntry>,
}

#[derive(Debug, Clone)]
pub(crate) struct CapabilityCatalog {
    pub(crate) source: &'static str,
    pub(crate) cache_key: String,
    pub(crate) profile_dir: String,
    pub(crate) workspace_key: String,
    pub(crate) report: Option<CapabilityReport>,
}

#[derive(Debug)]
pub(crate) struct CapabilityCatalogCache {
    path: PathBuf,
    inner: Mutex<CapabilityCacheState>,
}

impl CapabilityCatalogCache {
    pub(crate) fn load(config: &AppConfig) -> Result<Self> {
        let path = capability_cache_path(config);
        let state = if path.exists() {
            let raw =
                fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
            serde_json::from_str(&raw).context("decode capability cache")?
        } else {
            CapabilityCacheState::default()
        };
        Ok(Self {
            path,
            inner: Mutex::new(state),
        })
    }

    pub(crate) fn resolve(&self, core: &CoreService) -> Result<CapabilityCatalog> {
        let live = discover_capabilities(core);
        let profile = core.browser_profile_info().ok();
        let (cache_key, profile_dir, workspace_key) = catalog_identity(profile.as_ref(), core);

        if let Some(report) = live {
            self.remember(&cache_key, &profile_dir, &workspace_key, &report)?;
            return Ok(CapabilityCatalog {
                source: "live",
                cache_key,
                profile_dir,
                workspace_key,
                report: Some(report),
            });
        }

        if let Some(entry) = self.cached(&cache_key)? {
            return Ok(CapabilityCatalog {
                source: "cached",
                cache_key,
                profile_dir,
                workspace_key,
                report: Some(entry.report),
            });
        }

        Ok(CapabilityCatalog {
            source: "unknown",
            cache_key,
            profile_dir,
            workspace_key,
            report: None,
        })
    }

    pub(crate) fn cached(&self, cache_key: &str) -> Result<Option<CapabilityCacheEntry>> {
        let state = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("capability cache lock poisoned"))?;
        Ok(state.entries.get(cache_key).cloned())
    }

    pub(crate) fn remember(
        &self,
        cache_key: &str,
        profile_dir: &str,
        workspace_key: &str,
        report: &CapabilityReport,
    ) -> Result<()> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("capability cache lock poisoned"))?;
        state.entries.insert(
            cache_key.to_owned(),
            CapabilityCacheEntry {
                cache_key: cache_key.to_owned(),
                profile_dir: profile_dir.to_owned(),
                workspace_key: workspace_key.to_owned(),
                cached_at: Utc::now(),
                report: report.clone(),
            },
        );
        let raw = serde_json::to_vec_pretty(&*state).context("encode capability cache")?;
        fs::write(&self.path, raw).with_context(|| format!("write {}", self.path.display()))
    }
}

fn capability_cache_path(config: &AppConfig) -> PathBuf {
    config
        .paths
        .state_root
        .join("gateway-capability-cache.json")
}

fn catalog_identity(
    snapshot: Option<&BrowserStatusSnapshot>,
    core: &CoreService,
) -> (String, String, String) {
    let profile_dir = snapshot
        .map(|value| value.profile_dir.clone())
        .unwrap_or_else(|| {
            core.config()
                .paths
                .browser_profile_dir
                .display()
                .to_string()
        });
    let workspace_key = snapshot
        .and_then(|value| value.current_url.clone().or(Some(value.target_url.clone())))
        .unwrap_or_else(|| core.config().browser_target_url().to_owned());
    (
        format!("{profile_dir}::{workspace_key}"),
        profile_dir,
        workspace_key,
    )
}
