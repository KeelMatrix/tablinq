use anyhow::Result;
use gptd::{AppConfig, CoreService};
use shared_protocol::BrowserOpenRequest;
use std::thread;
use std::time::{Duration, Instant};

pub(crate) fn ensure_browser_session(core: &CoreService) -> Result<()> {
    let mut snapshot = core.browser_profile_info()?;
    if !snapshot.running {
        core.browser_open(BrowserOpenRequest {
            target_url: None,
            browser_path: None,
            headless: Some(true),
        })?;
        snapshot = core.browser_profile_info()?;
    }
    if !snapshot.authenticated {
        anyhow::bail!("auth required: attached browser-backed ChatGPT tab is not authenticated");
    }
    Ok(())
}

pub(crate) fn discover_capabilities(
    core: &CoreService,
) -> Option<shared_protocol::CapabilityReport> {
    let needs_browser = match core.browser_profile_info() {
        Ok(snapshot) => !snapshot.running,
        Err(_) => true,
    };
    if needs_browser {
        let _ = core.browser_open(BrowserOpenRequest {
            target_url: None,
            browser_path: None,
            headless: Some(true),
        });
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut last_report = None;
    loop {
        if let Ok(snapshot) = core.capabilities()
            && let Some(report) = snapshot.report
        {
            if capability_report_is_usable(&report) {
                return Some(report);
            }
            last_report = Some(report);
        }

        if Instant::now() >= deadline {
            return last_report;
        }

        thread::sleep(Duration::from_millis(100));
    }
}

pub(crate) fn capability_report_is_usable(report: &shared_protocol::CapabilityReport) -> bool {
    let advertises_search = report.aliases.modes.contains_key("search");
    let advertises_research = report.aliases.modes.contains_key("research");
    let advertises_thinking = report.aliases.model_classes.contains_key("thinking");

    report.matrix.composer_ready
        && !report.aliases.modes.is_empty()
        && !report.aliases.model_classes.is_empty()
        && (!advertises_search || report.matrix.search_mode_visible)
        && (!advertises_research || report.matrix.research_mode_visible)
        && (!advertises_thinking
            || report.matrix.model_picker_visible
            || report.matrix.thinking_effort_visible)
        && report.matrix.upload_control_visible
}

pub(crate) fn recommended_cli_browser_setup(config: &AppConfig) -> String {
    if matches!(
        config.browser_connection_mode(),
        shared_protocol::BrowserConnectionMode::AttachFirst
            | shared_protocol::BrowserConnectionMode::ManagedFallback
    ) {
        "Use `gpt browser start-debug` from the CLI to launch the local debug browser, then call `/v1/browser/attach` or `gpt browser attach`.".to_owned()
    } else {
        "CLI `gpt browser start-debug` remains the operator-owned setup path for local debug launches; managed-only mode can use `gpt browser open` as a fallback.".to_owned()
    }
}
