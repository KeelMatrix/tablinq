use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use shared_protocol::{
    CONFIG_VERSION, ConfigSnapshot, StatePaths, build_state_paths, resolve_state_root,
};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub version: Option<u32>,
    pub warm_mode: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub paths: StatePaths,
    pub file: FileConfig,
}

impl AppConfig {
    pub fn load_or_create(explicit_root: Option<&Path>) -> Result<Self> {
        let state_root = resolve_state_root(explicit_root);
        let paths = build_state_paths(state_root);
        ensure_directories(&paths)?;

        let file = load_or_create_file_config(&paths.config_path)?;

        Ok(Self { paths, file })
    }

    pub fn snapshot(&self) -> ConfigSnapshot {
        ConfigSnapshot {
            version: CONFIG_VERSION,
            state_root: self.paths.state_root.display().to_string(),
            config_dir: self.paths.config_dir.display().to_string(),
            config_path: self.paths.config_path.display().to_string(),
            logs_dir: self.paths.logs_dir.display().to_string(),
            log_path: self.paths.log_path.display().to_string(),
            db_dir: self.paths.db_dir.display().to_string(),
            db_path: self.paths.db_path.display().to_string(),
            browser_profile_dir: self.paths.browser_profile_dir.display().to_string(),
            downloads_dir: self.paths.downloads_dir.display().to_string(),
            crash_bundles_dir: self.paths.crash_bundles_dir.display().to_string(),
            ipc_endpoint: self.paths.ipc_endpoint.clone(),
            warm_mode: self.warm_mode(),
        }
    }

    pub fn warm_mode(&self) -> bool {
        self.file.warm_mode.unwrap_or(false)
    }
}

pub fn parse_file_config(raw: &str) -> Result<FileConfig> {
    let parsed: FileConfig = toml::from_str(raw).context("parse config.toml")?;
    Ok(upgrade_file_config(parsed))
}

fn load_or_create_file_config(path: &Path) -> Result<FileConfig> {
    if path.exists() {
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let file = parse_file_config(&raw)?;
        write_file_config(path, &file)?;
        Ok(file)
    } else {
        let file = upgrade_file_config(FileConfig::default());
        write_file_config(path, &file)?;
        Ok(file)
    }
}

fn write_file_config(path: &Path, file: &FileConfig) -> Result<()> {
    let raw = toml::to_string_pretty(file).context("serialize config.toml")?;
    fs::write(path, raw).with_context(|| format!("write {}", path.display()))
}

fn ensure_directories(paths: &StatePaths) -> Result<()> {
    for dir in [
        &paths.state_root,
        &paths.config_dir,
        &paths.logs_dir,
        &paths.db_dir,
        &paths.browser_profile_dir,
        &paths.downloads_dir,
        &paths.crash_bundles_dir,
    ] {
        fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    Ok(())
}

fn upgrade_file_config(mut file: FileConfig) -> FileConfig {
    file.version = Some(CONFIG_VERSION);
    if file.warm_mode.is_none() {
        file.warm_mode = Some(false);
    }
    file
}

pub fn resolve_state_root_from_arg(path: Option<PathBuf>) -> Result<PathBuf> {
    let state_root = resolve_state_root(path.as_deref());
    if state_root.as_os_str().is_empty() {
        bail!("state root must not be empty");
    }
    Ok(state_root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use tempfile::tempdir;

    #[test]
    fn load_or_create_creates_config_and_directories() {
        let dir = tempdir().unwrap();
        let config = AppConfig::load_or_create(Some(dir.path())).unwrap();

        assert!(config.paths.config_path.exists());
        assert!(config.paths.logs_dir.exists());
        assert!(config.paths.db_dir.exists());
        assert_eq!(config.file.version, Some(CONFIG_VERSION));
        assert_eq!(config.file.warm_mode, Some(false));
    }

    #[test]
    fn parse_file_config_upgrades_missing_values() {
        let parsed = parse_file_config("version = 0").unwrap();
        assert_eq!(parsed.version, Some(CONFIG_VERSION));
        assert_eq!(parsed.warm_mode, Some(false));
    }

    proptest! {
        #[test]
        fn file_config_upgrade_preserves_explicit_warm_mode(
            version in proptest::option::of(0u32..10),
            warm_mode in proptest::option::of(any::<bool>()),
        ) {
            let file = FileConfig { version, warm_mode };
            let upgraded = super::upgrade_file_config(file.clone());

            prop_assert_eq!(upgraded.version, Some(CONFIG_VERSION));
            prop_assert_eq!(upgraded.warm_mode, Some(file.warm_mode.unwrap_or(false)));
        }
    }
}
