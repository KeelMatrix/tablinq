use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use shared_protocol::{
    BrowserConnectionMode, CONFIG_VERSION, ConfigSnapshot, GatewayConfigSnapshot, StatePaths,
    build_state_paths, resolve_state_root,
};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct GatewayFileConfig {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub api_key: Option<String>,
    pub max_concurrent_upstream_jobs: Option<u32>,
    pub queue_length: Option<u32>,
    pub queue_timeout_ms: Option<u64>,
    pub per_client_limit: Option<u32>,
    pub ask_timeout_ms: Option<u64>,
    pub extended_thinking_timeout_ms: Option<u64>,
    pub research_timeout_ms: Option<u64>,
    pub safe_mode_behavior: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub version: Option<u32>,
    pub warm_mode: Option<bool>,
    pub safe_mode: Option<bool>,
    pub browser_connection_mode: Option<BrowserConnectionMode>,
    pub browser_debug_endpoint: Option<String>,
    pub browser_debug_host: Option<String>,
    pub browser_debug_port: Option<u16>,
    pub browser_channel: Option<String>,
    pub browser_target_url: Option<String>,
    pub browser_path: Option<String>,
    pub browser_extension_required: Option<bool>,
    pub browser_auto_open_target_tab: Option<bool>,
    pub gateway: Option<GatewayFileConfig>,
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
            chrome_attach_profile_dir: self.paths.chrome_attach_profile_dir.display().to_string(),
            downloads_dir: self.paths.downloads_dir.display().to_string(),
            crash_bundles_dir: self.paths.crash_bundles_dir.display().to_string(),
            ipc_endpoint: self.paths.ipc_endpoint.clone(),
            warm_mode: self.warm_mode(),
            safe_mode: self.safe_mode(),
            browser_target_url: self.browser_target_url().to_owned(),
            browser_path: self.browser_path().map(ToOwned::to_owned),
            browser_connection_mode: self.browser_connection_mode(),
            browser_debug_endpoint: self.browser_debug_endpoint(),
            browser_debug_host: self.browser_debug_host().to_owned(),
            browser_debug_port: self.browser_debug_port(),
            browser_channel: self.browser_channel().to_owned(),
            browser_extension_required: self.browser_extension_required(),
            browser_auto_open_target_tab: self.browser_auto_open_target_tab(),
            gateway: GatewayConfigSnapshot {
                host: self.gateway_host().to_owned(),
                port: self.gateway_port(),
                api_key_present: self.gateway_api_key().is_some(),
                max_concurrent_upstream_jobs: self.gateway_max_concurrent_upstream_jobs(),
                queue_length: self.gateway_queue_length(),
                queue_timeout_ms: self.gateway_queue_timeout_ms(),
                per_client_limit: self.gateway_per_client_limit(),
                request_timeout_ms: self.gateway_request_timeout_ms(),
                ask_timeout_ms: self.ask_timeout_ms(),
                extended_thinking_timeout_ms: self.extended_thinking_timeout_ms(),
                research_timeout_ms: self.research_timeout_ms(),
                safe_mode_behavior: self.gateway_safe_mode_behavior().to_owned(),
            },
        }
    }

    pub fn warm_mode(&self) -> bool {
        self.file.warm_mode.unwrap_or(false)
    }

    pub fn safe_mode(&self) -> bool {
        self.file.safe_mode.unwrap_or(false)
    }

    pub fn browser_connection_mode(&self) -> BrowserConnectionMode {
        self.file
            .browser_connection_mode
            .unwrap_or(BrowserConnectionMode::AttachFirst)
    }

    pub fn browser_debug_endpoint(&self) -> String {
        if let Some(value) = self
            .file
            .browser_debug_endpoint
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            return value.to_owned();
        }

        format!(
            "http://{}:{}",
            self.browser_debug_host(),
            self.browser_debug_port()
        )
    }

    pub fn browser_debug_host(&self) -> &str {
        self.file
            .browser_debug_host
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("127.0.0.1")
    }

    pub fn browser_debug_port(&self) -> u16 {
        self.file.browser_debug_port.unwrap_or(9222)
    }

    pub fn browser_channel(&self) -> &str {
        self.file
            .browser_channel
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(if cfg!(windows) { "msedge" } else { "chrome" })
    }

    pub fn browser_target_url(&self) -> &str {
        self.file
            .browser_target_url
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("https://chatgpt.com/")
    }

    pub fn browser_path(&self) -> Option<&str> {
        self.file
            .browser_path
            .as_deref()
            .filter(|value| !value.trim().is_empty())
    }

    pub fn browser_extension_required(&self) -> bool {
        self.file.browser_extension_required.unwrap_or(true)
    }

    pub fn browser_auto_open_target_tab(&self) -> bool {
        self.file.browser_auto_open_target_tab.unwrap_or(true)
    }

    pub fn gateway_host(&self) -> &str {
        self.file
            .gateway
            .as_ref()
            .and_then(|value| value.host.as_deref())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("127.0.0.1")
    }

    pub fn gateway_port(&self) -> u16 {
        self.file
            .gateway
            .as_ref()
            .and_then(|value| value.port)
            .unwrap_or(3847)
    }

    pub fn gateway_api_key(&self) -> Option<&str> {
        self.file
            .gateway
            .as_ref()
            .and_then(|value| value.api_key.as_deref())
            .filter(|value| !value.trim().is_empty())
    }

    pub fn gateway_max_concurrent_upstream_jobs(&self) -> u32 {
        self.file
            .gateway
            .as_ref()
            .and_then(|value| value.max_concurrent_upstream_jobs)
            .unwrap_or(1)
    }

    pub fn gateway_queue_length(&self) -> u32 {
        self.file
            .gateway
            .as_ref()
            .and_then(|value| value.queue_length)
            .unwrap_or(32)
    }

    pub fn gateway_queue_timeout_ms(&self) -> u64 {
        self.file
            .gateway
            .as_ref()
            .and_then(|value| value.queue_timeout_ms)
            .unwrap_or(4_800_000)
    }

    pub fn gateway_per_client_limit(&self) -> u32 {
        self.file
            .gateway
            .as_ref()
            .and_then(|value| value.per_client_limit)
            .unwrap_or(8)
    }

    pub fn gateway_request_timeout_ms(&self) -> u64 {
        self.ask_timeout_ms()
            .max(self.extended_thinking_timeout_ms())
            .max(self.research_timeout_ms())
    }

    pub fn ask_timeout_ms(&self) -> u64 {
        self.file
            .gateway
            .as_ref()
            .and_then(|value| value.ask_timeout_ms)
            .unwrap_or(15_000)
    }

    pub fn extended_thinking_timeout_ms(&self) -> u64 {
        self.file
            .gateway
            .as_ref()
            .and_then(|value| value.extended_thinking_timeout_ms)
            .unwrap_or(1_500_000)
    }

    pub fn research_timeout_ms(&self) -> u64 {
        self.file
            .gateway
            .as_ref()
            .and_then(|value| value.research_timeout_ms)
            .unwrap_or(30_000)
    }

    pub fn gateway_safe_mode_behavior(&self) -> &str {
        self.file
            .gateway
            .as_ref()
            .and_then(|value| value.safe_mode_behavior.as_deref())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("reject_execution")
    }
}

pub fn parse_file_config(raw: &str) -> Result<FileConfig> {
    let parsed: FileConfig = toml::from_str(raw).context("parse config.toml")?;
    ensure_current_config_version(&parsed)?;
    Ok(normalize_file_config(parsed))
}

fn load_or_create_file_config(path: &Path) -> Result<FileConfig> {
    if path.exists() {
        let raw = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        let file = parse_file_config(&raw)?;
        write_file_config(path, &file)?;
        Ok(file)
    } else {
        let file = normalize_file_config(FileConfig::default());
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
        &paths.chrome_attach_profile_dir,
        &paths.downloads_dir,
        &paths.crash_bundles_dir,
    ] {
        fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    Ok(())
}

fn ensure_current_config_version(file: &FileConfig) -> Result<()> {
    match file.version {
        Some(CONFIG_VERSION) => Ok(()),
        Some(version) => {
            bail!("config.toml version {version} is unsupported; expected version {CONFIG_VERSION}")
        }
        None => bail!(
            "config.toml must declare version = {CONFIG_VERSION}; older config layouts are unsupported"
        ),
    }
}

fn normalize_file_config(mut file: FileConfig) -> FileConfig {
    file.version = Some(CONFIG_VERSION);
    if file.warm_mode.is_none() {
        file.warm_mode = Some(false);
    }
    if file.safe_mode.is_none() {
        file.safe_mode = Some(false);
    }
    if file.browser_connection_mode.is_none() {
        file.browser_connection_mode = Some(BrowserConnectionMode::AttachFirst);
    }
    if file.browser_debug_host.is_none() {
        file.browser_debug_host = Some("127.0.0.1".to_owned());
    }
    if file.browser_debug_port.is_none() {
        file.browser_debug_port = Some(9222);
    }
    if file.browser_channel.is_none() {
        file.browser_channel = Some(if cfg!(windows) {
            "msedge".to_owned()
        } else {
            "chrome".to_owned()
        });
    }
    if file.browser_target_url.is_none() {
        file.browser_target_url = Some("https://chatgpt.com/".to_owned());
    }
    if file.browser_extension_required.is_none() {
        file.browser_extension_required = Some(true);
    }
    if file.browser_auto_open_target_tab.is_none() {
        file.browser_auto_open_target_tab = Some(true);
    }
    let gateway = file.gateway.get_or_insert_with(GatewayFileConfig::default);
    if gateway.host.is_none() {
        gateway.host = Some("127.0.0.1".to_owned());
    }
    if gateway.port.is_none() {
        gateway.port = Some(3847);
    }
    if gateway.api_key.is_none() {
        gateway.api_key = Some(generate_gateway_api_key());
    }
    if gateway.max_concurrent_upstream_jobs.is_none() {
        gateway.max_concurrent_upstream_jobs = Some(1);
    }
    if gateway.queue_length.is_none() {
        gateway.queue_length = Some(32);
    }
    if gateway.queue_timeout_ms.is_none() {
        gateway.queue_timeout_ms = Some(4_800_000);
    }
    if gateway.per_client_limit.is_none() {
        gateway.per_client_limit = Some(8);
    }
    if gateway.ask_timeout_ms.is_none() {
        gateway.ask_timeout_ms = Some(360_000);
    }
    if gateway.extended_thinking_timeout_ms.is_none() {
        gateway.extended_thinking_timeout_ms = Some(1_500_000);
    }
    if gateway.research_timeout_ms.is_none() {
        gateway.research_timeout_ms = Some(4_800_000);
    }
    if gateway.safe_mode_behavior.is_none() {
        gateway.safe_mode_behavior = Some("reject_execution".to_owned());
    }
    file
}

fn generate_gateway_api_key() -> String {
    format!(
        "tablinq-local-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_micros()
    )
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
        assert_eq!(config.file.safe_mode, Some(false));
        assert_eq!(
            config.file.browser_connection_mode,
            Some(BrowserConnectionMode::AttachFirst)
        );
        assert_eq!(config.file.browser_debug_host.as_deref(), Some("127.0.0.1"));
        assert_eq!(config.file.browser_debug_port, Some(9222));
        assert_eq!(
            config.browser_channel(),
            if cfg!(windows) { "msedge" } else { "chrome" }
        );
        assert_eq!(
            config.file.browser_target_url.as_deref(),
            Some("https://chatgpt.com/")
        );
        assert!(config.browser_extension_required());
        assert!(config.browser_auto_open_target_tab());
        assert_eq!(config.gateway_host(), "127.0.0.1");
        assert_eq!(config.gateway_port(), 3847);
        assert_eq!(config.gateway_max_concurrent_upstream_jobs(), 1);
        assert_eq!(config.gateway_queue_length(), 32);
        assert_eq!(config.gateway_queue_timeout_ms(), 4_800_000);
        assert_eq!(config.gateway_per_client_limit(), 8);
        assert_eq!(config.gateway_request_timeout_ms(), 4_800_000);
        assert_eq!(config.ask_timeout_ms(), 360_000);
        assert_eq!(config.extended_thinking_timeout_ms(), 1_500_000);
        assert_eq!(config.research_timeout_ms(), 4_800_000);
        assert_eq!(config.gateway_safe_mode_behavior(), "reject_execution");
        assert!(config.gateway_api_key().is_some());
    }

    #[test]
    fn parse_file_config_fills_missing_values_for_current_version() {
        let parsed = parse_file_config(&format!("version = {CONFIG_VERSION}")).unwrap();
        assert_eq!(parsed.version, Some(CONFIG_VERSION));
        assert_eq!(parsed.warm_mode, Some(false));
        assert_eq!(parsed.safe_mode, Some(false));
        assert_eq!(
            parsed.browser_connection_mode,
            Some(BrowserConnectionMode::AttachFirst)
        );
        assert_eq!(parsed.browser_debug_port, Some(9222));
        assert_eq!(
            parsed.browser_target_url.as_deref(),
            Some("https://chatgpt.com/")
        );
        assert_eq!(
            parsed.gateway.as_ref().and_then(|value| value.port),
            Some(3847)
        );
        assert_eq!(
            parsed
                .gateway
                .as_ref()
                .and_then(|value| value.max_concurrent_upstream_jobs),
            Some(1)
        );
        assert_eq!(
            parsed
                .gateway
                .as_ref()
                .and_then(|value| value.queue_timeout_ms),
            Some(4_800_000)
        );
        assert_eq!(
            parsed
                .gateway
                .as_ref()
                .and_then(|value| value.per_client_limit),
            Some(8)
        );
        assert_eq!(
            parsed
                .gateway
                .as_ref()
                .and_then(|value| value.extended_thinking_timeout_ms),
            Some(1_500_000)
        );
        assert_eq!(
            parsed
                .gateway
                .as_ref()
                .and_then(|value| value.research_timeout_ms),
            Some(4_800_000)
        );
        assert!(
            parsed
                .gateway
                .as_ref()
                .and_then(|value| value.api_key.as_deref())
                .is_some()
        );
    }

    #[test]
    fn parse_file_config_rejects_missing_version() {
        let err = parse_file_config("warm_mode = true").unwrap_err();
        assert!(format!("{err:#}").contains(&format!(
            "config.toml must declare version = {CONFIG_VERSION}"
        )));
    }

    #[test]
    fn parse_file_config_rejects_outdated_version() {
        let err = parse_file_config(
            r#"
version = 1
warm_mode = true
"#,
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("config.toml version 1 is unsupported"));
    }

    proptest! {
        #[test]
        fn file_config_normalization_preserves_explicit_warm_mode(
            warm_mode in proptest::option::of(any::<bool>()),
        ) {
            let file = FileConfig {
                version: Some(CONFIG_VERSION),
                warm_mode,
                safe_mode: None,
                browser_connection_mode: None,
                browser_debug_endpoint: None,
                browser_debug_host: None,
                browser_debug_port: None,
                browser_channel: None,
                browser_target_url: None,
                browser_path: None,
                browser_extension_required: None,
                browser_auto_open_target_tab: None,
                gateway: None,
            };
            let normalized = super::normalize_file_config(file.clone());

            prop_assert_eq!(normalized.version, Some(CONFIG_VERSION));
            prop_assert_eq!(normalized.warm_mode, Some(file.warm_mode.unwrap_or(false)));
        }
    }
}
