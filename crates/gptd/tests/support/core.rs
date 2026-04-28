use gptd::AppConfig;
use shared_protocol::BrowserConnectionMode;
use std::path::Path;

pub fn core_browser_integration_prereqs_available() -> bool {
    gptd::browser::browser_integration_prereqs_available()
}

pub fn core_test_config(dir: &Path) -> AppConfig {
    let mut config = AppConfig::load_or_create(Some(dir)).unwrap();
    config.file.browser_connection_mode = Some(BrowserConnectionMode::ManagedOnly);
    config.file.browser_submit_jitter_min_ms = Some(0);
    config.file.browser_submit_jitter_max_ms = Some(0);
    if let Some(gateway) = config.file.gateway.as_mut() {
        gateway.ask_timeout_ms = Some(60_000);
        gateway.extended_thinking_timeout_ms = Some(60_000);
        gateway.research_timeout_ms = Some(120_000);
    }
    config
}
