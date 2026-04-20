use super::{CoreService, ExecutionRecoveryState};
use crate::browser::{
    BrowserDiagnosticsSnapshot, BrowserManager, RequestLifecycleUpdate, ResearchUpdate,
};
use crate::config::AppConfig;
use crate::db::Database;
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use shared_protocol::{
    BrowserConnectionMode, BrowserStatusSnapshot, ErrorCode, EventPayload, HealthCheck,
    HealthStatus, JobState, JobStatus, ResearchCommand, SelectorPackSummary, StreamChunk,
    StreamChunkKind,
};
use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use url::Url;

impl CoreService {
    pub(super) fn open_db(&self) -> Result<Database> {
        Database::open(&self.config.paths.db_path)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_request_lifecycle_update<F>(
        &self,
        db: &Database,
        job_id: &str,
        created_at: DateTime<Utc>,
        conversation_id: Option<&str>,
        update: RequestLifecycleUpdate,
        recovery_state: &RefCell<ExecutionRecoveryState>,
        on_event: &RefCell<&mut F>,
    ) -> Result<()>
    where
        F: FnMut(EventPayload) -> Result<()>,
    {
        let updated_at = Utc::now();
        match update.status {
            JobStatus::Interrupted => {
                recovery_state.borrow_mut().interrupted = true;
                db.insert_job(
                    job_id,
                    JobStatus::Interrupted,
                    Some(&update.message),
                    update.conversation_id.as_deref().or(conversation_id),
                    created_at,
                    updated_at,
                )?;
                db.record_job_interruption(
                    job_id,
                    updated_at,
                    &update.message,
                    update.current_url.as_deref(),
                )?;
            }
            JobStatus::Recovered => {
                recovery_state.borrow_mut().recovered = true;
                db.insert_job(
                    job_id,
                    JobStatus::Recovered,
                    Some(&update.message),
                    update.conversation_id.as_deref().or(conversation_id),
                    created_at,
                    updated_at,
                )?;
                db.record_job_recovery(
                    job_id,
                    updated_at,
                    &update.message,
                    update.current_url.as_deref(),
                )?;
            }
            _ => return Ok(()),
        }

        let mut sink = on_event.borrow_mut();
        (*sink)(EventPayload::JobState(JobState {
            job_id: job_id.to_owned(),
            status: update.status,
            message: Some(update.message),
            conversation_id: update.conversation_id,
            updated_at,
            artifacts: vec![],
        }))
    }
}

pub fn error_code_for_message(message: &str) -> ErrorCode {
    if message.contains("profile locked") || message.contains("SingletonLock") {
        ErrorCode::ProfileLocked
    } else if message.contains("auth required") {
        ErrorCode::AuthRequired
    } else if message.contains("browser launch failed")
        || message.contains("spawn browser adapter")
        || message.contains("supported browser executable")
    {
        ErrorCode::BrowserLaunch
    } else if message.contains("capability missing") {
        ErrorCode::CapabilityMissing
    } else if message.contains("research job failed")
        || message.contains("research failed")
        || message.contains("research cancelled")
    {
        ErrorCode::ResearchJobFailed
    } else if message.contains("upload failed") {
        ErrorCode::UploadFailed
    } else if message.contains("download failed") {
        ErrorCode::DownloadFailed
    } else if message.contains("ui drift") {
        ErrorCode::UiDrift
    } else if message.contains("timed out") {
        ErrorCode::ActionTimeout
    } else {
        ErrorCode::Transport
    }
}

pub(super) fn lock_browser<'a>(
    browser: &'a Arc<Mutex<BrowserManager>>,
) -> Result<MutexGuard<'a, BrowserManager>> {
    browser
        .lock()
        .map_err(|_| anyhow!("browser manager lock poisoned"))
}

pub(super) fn try_lock_browser<'a>(
    browser: &'a Arc<Mutex<BrowserManager>>,
) -> Result<Option<MutexGuard<'a, BrowserManager>>> {
    match browser.try_lock() {
        Ok(guard) => Ok(Some(guard)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Poisoned(_)) => Err(anyhow!("browser manager lock poisoned")),
    }
}

pub(super) fn lock_cancel_registry<'a>(
    registry: &'a Arc<Mutex<HashSet<String>>>,
) -> Result<MutexGuard<'a, HashSet<String>>> {
    registry
        .lock()
        .map_err(|_| anyhow!("cancel registry lock poisoned"))
}

pub(super) fn is_cancelled_message(message: &str) -> bool {
    message.contains("cancelled")
}

pub(super) fn browser_busy_message() -> &'static str {
    "browser busy: another ask or research request is already using the browser session"
}

pub(super) fn resolve_conversation_target_url(base_url: &str, conversation_id: &str) -> String {
    let normalized_conversation_id = conversation_id.trim();
    if normalized_conversation_id.is_empty() {
        return base_url.to_owned();
    }

    let Ok(mut url) = Url::parse(base_url) else {
        return base_url.to_owned();
    };
    let mut segments = url
        .path_segments()
        .map(|parts| parts.map(str::to_owned).collect::<Vec<_>>())
        .unwrap_or_default();
    if segments.len() >= 2
        && segments[segments.len() - 2].eq_ignore_ascii_case("c")
        && !segments[segments.len() - 1].is_empty()
    {
        segments.truncate(segments.len() - 2);
    }

    let Ok(mut path_segments) = url.path_segments_mut() else {
        return base_url.to_owned();
    };
    path_segments.clear();
    for segment in &segments {
        path_segments.push(segment);
    }
    path_segments.push("c");
    path_segments.push(normalized_conversation_id);
    drop(path_segments);

    url.to_string()
}

pub(super) fn busy_health_snapshot() -> HealthCheck {
    HealthCheck {
        status: HealthStatus::Degraded,
        checked_at: Utc::now(),
        browser_connected: false,
        extension_connected: false,
        authenticated: false,
        details: Some(browser_busy_message().to_owned()),
    }
}

pub(super) fn health_snapshot(config: &AppConfig, browser: BrowserStatusSnapshot) -> HealthCheck {
    let status = if !browser.running || !browser.extension_connected || !browser.authenticated {
        HealthStatus::Degraded
    } else {
        HealthStatus::Ok
    };
    HealthCheck {
        status,
        checked_at: Utc::now(),
        browser_connected: browser.running,
        extension_connected: browser.extension_connected,
        authenticated: browser.authenticated,
        details: Some(if config.warm_mode() {
            format!(
                "control plane ready; warm mode enabled; attach_mode={:?}; browser_running={}; extension_connected={}",
                browser.attach_mode, browser.running, browser.extension_connected
            )
        } else {
            browser_health_details(&browser, config)
        }),
    }
}

pub(super) fn recovery_guidance(
    config: &AppConfig,
    browser: &BrowserStatusSnapshot,
    diagnostics: Option<&BrowserDiagnosticsSnapshot>,
) -> Vec<String> {
    let mut guidance = Vec::new();
    let mut attach_problem = false;
    let debug_endpoint = browser
        .debug_endpoint
        .clone()
        .unwrap_or_else(|| config.browser_debug_endpoint());
    if config.safe_mode() {
        guidance.push(
            "safe mode is enabled; ask and research are disabled until config.toml is updated"
                .to_owned(),
        );
    }
    if is_attach_primary_mode(browser.attach_mode) {
        if !browser.running {
            attach_problem = true;
            guidance.push(format!(
                "attach endpoint is not reachable at {debug_endpoint}; fully close Chrome or Edge, run `gpt browser start-debug`, then rerun `gpt browser attach`"
            ));
            guidance.push(
                "run `gpt browser connection-info` and confirm the configured debug endpoint, target URL, and browser path before switching modes"
                    .to_owned(),
            );
        }
        if browser.extension_required && !browser.extension_connected {
            attach_problem = true;
            guidance.push(
                "the Tablinq extension is missing or disconnected in the attached browser profile; confirm it is installed, enabled, and running in the same browser profile Tablinq is attaching to"
                    .to_owned(),
            );
        }
        if current_url_requires_login_or_challenge(browser.current_url.as_deref()) {
            attach_problem = true;
            guidance.push(
                "the attached ChatGPT tab is blocked on sign-in or a challenge page; complete that page in your normal browser, then rerun `gpt browser attach`"
                    .to_owned(),
            );
        } else if !browser.authenticated {
            attach_problem = true;
            guidance.push(
                "the attached ChatGPT tab is signed out; sign in to ChatGPT in your normal browser, then rerun `gpt browser attach`"
                    .to_owned(),
            );
        }
        if !browser.target_tab_found {
            attach_problem = true;
            guidance.push(
                "target tab not found; run `gpt browser open` to create or focus the configured ChatGPT tab after the browser is signed in"
                    .to_owned(),
            );
        }
        if browser.running
            && (!browser.target_tab_found
                || !browser.authenticated
                || (browser.extension_required && !browser.extension_connected))
        {
            guidance.push(
                "if multiple ChatGPT tabs are open, keep only the signed-in tab with the working Tablinq extension, then rerun `gpt browser attach`"
                    .to_owned(),
            );
        }
        if attach_problem {
            guidance.push(
                "only if your machine or browser profile cannot expose the local debug endpoint, or you need debugging or CI/local harness isolation, set `browser_connection_mode = \"managed_only\"` in config.toml and use `gpt browser open` as explicit fallback"
                    .to_owned(),
            );
        }
        if browser.running
            && browser.target_tab_found
            && browser.authenticated
            && (!browser.extension_required || browser.extension_connected)
        {
            guidance.push("attached browser session is ready".to_owned());
        }
    } else {
        if !browser.running {
            guidance.push(
                "managed browser fallback is fallback/debug-only; use `gpt browser open` to start the managed browser tab"
                    .to_owned(),
            );
        } else {
            guidance.push(
                "daemon can recreate the managed fallback tab and relaunch the browser adapter when the page crashes or closes"
                    .to_owned(),
            );
        }
        guidance.push(
            "use managed fallback for debugging, CI/local harness isolation, or when attach-first cannot expose a local debug endpoint"
                .to_owned(),
        );
        if !browser.authenticated {
            guidance.push(
                "managed browser fallback is signed out; open the managed browser and sign in before running ask or research"
                    .to_owned(),
            );
        }
        if !browser.extension_connected {
            guidance.push(
                "extension probe is disconnected in managed fallback mode; reload the page or relaunch the browser to refresh capabilities"
                    .to_owned(),
            );
        }
    }
    if let Some(diagnostics) = diagnostics {
        if !diagnostics.browser_detected {
            guidance.push(
                "no supported browser was detected; set an explicit browser path in config.toml"
                    .to_owned(),
            );
        }
        if !diagnostics.profile_accessible && supports_managed_profile_reset(browser.attach_mode) {
            guidance.push(
                "browser profile is not writable; close other managed browser instances or run `gpt browser reset-profile`"
                    .to_owned(),
            );
        }
    }
    guidance
}

pub(super) fn current_url_requires_login_or_challenge(current_url: Option<&str>) -> bool {
    let Some(current_url) = current_url else {
        return false;
    };
    let lowered = current_url.to_ascii_lowercase();
    ["challenge", "captcha", "login", "signin", "auth", "cf_chl"]
        .iter()
        .any(|marker| lowered.contains(marker))
}

fn browser_health_details(browser: &BrowserStatusSnapshot, config: &AppConfig) -> String {
    if !browser.running {
        return if is_attach_primary_mode(browser.attach_mode) {
            format!(
                "control plane ready; attach-first browser not connected; debug_endpoint={}",
                browser
                    .debug_endpoint
                    .clone()
                    .unwrap_or_else(|| config.browser_debug_endpoint())
            )
        } else {
            "control plane ready; managed browser fallback not started yet".to_owned()
        };
    }

    if is_attach_primary_mode(browser.attach_mode) {
        if !browser.target_tab_found {
            return "control plane ready; attached browser is connected but the target tab was not found"
                .to_owned();
        }
        return match browser.current_url.as_deref() {
            Some(url) => format!(
                "control plane ready; attached tab at {url}; authenticated={}; extension_connected={}",
                browser.authenticated, browser.extension_connected
            ),
            None => format!(
                "control plane ready; attached browser is connected; authenticated={}; extension_connected={}",
                browser.authenticated, browser.extension_connected
            ),
        };
    }

    match browser.current_url.as_deref() {
        Some(url) => format!(
            "control plane ready; managed fallback tab at {url}; extension_connected={}",
            browser.extension_connected
        ),
        None => "control plane ready; managed browser fallback is connected".to_owned(),
    }
}

pub(super) fn is_attach_primary_mode(mode: BrowserConnectionMode) -> bool {
    matches!(
        mode,
        BrowserConnectionMode::AttachFirst | BrowserConnectionMode::ManagedFallback
    )
}

pub(super) fn supports_managed_profile_reset(mode: BrowserConnectionMode) -> bool {
    matches!(
        mode,
        BrowserConnectionMode::ManagedFallback | BrowserConnectionMode::ManagedOnly
    )
}

pub(super) fn browser_session_identity(snapshot: &BrowserStatusSnapshot) -> (String, String) {
    if snapshot.cdp_attached
        && let Some(tab_id) = snapshot
            .target_tab_id
            .as_deref()
            .filter(|value| !value.trim().is_empty())
    {
        let short_id = &tab_id[..tab_id.len().min(8)];
        return (
            format!("browser-tab-{tab_id}"),
            format!("Browser tab {short_id}"),
        );
    }

    ("browser-primary".to_owned(), "Browser session".to_owned())
}

pub(super) fn default_selector_pack_summary() -> Option<SelectorPackSummary> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("selector-packs")
        .join("v1")
        .join("chatgpt-default.json");
    let raw = fs::read_to_string(&path).ok()?;
    let parsed: shared_protocol::SelectorPack = serde_json::from_str(&raw).ok()?;
    Some(SelectorPackSummary {
        pack_id: parsed.pack_id,
        version: format!("v{}", parsed.format_version),
        updated_at: Some(parsed.updated_at),
        path: Some(path.display().to_string()),
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn apply_research_update<F>(
    db: &Database,
    job_id: &str,
    created_at: chrono::DateTime<Utc>,
    conversation_id: &mut Option<String>,
    current_url: &mut Option<String>,
    report_text: &mut String,
    next_seq: &mut u64,
    update: ResearchUpdate,
    on_event: &mut F,
) -> Result<()>
where
    F: FnMut(EventPayload) -> Result<()>,
{
    let updated_at = Utc::now();
    if let Some(value) = update.conversation_id.clone() {
        *conversation_id = Some(value);
    }
    if let Some(value) = update.current_url.clone() {
        *current_url = Some(value);
    }

    db.insert_job(
        job_id,
        update.status,
        update.message.as_deref(),
        update
            .conversation_id
            .as_deref()
            .or(conversation_id.as_deref()),
        created_at,
        updated_at,
    )?;

    if let Some(chunk_text) = update.chunk_text.clone() {
        if matches!(
            update.chunk_kind.unwrap_or(StreamChunkKind::Delta),
            StreamChunkKind::Delta
        ) {
            report_text.push_str(&chunk_text);
        }
        on_event(EventPayload::StreamChunk(StreamChunk {
            job_id: job_id.to_owned(),
            conversation_id: update.conversation_id.clone().or(conversation_id.clone()),
            message_id: update.message_id.clone(),
            seq: *next_seq,
            kind: update.chunk_kind.unwrap_or(StreamChunkKind::Delta),
            text: chunk_text,
        }))?;
        *next_seq += 1;
    }

    db.update_job_output(
        job_id,
        if report_text.is_empty() {
            None
        } else {
            Some(report_text.as_str())
        },
        current_url.as_deref(),
        None,
    )?;
    on_event(EventPayload::JobState(JobState {
        job_id: job_id.to_owned(),
        status: update.status,
        message: update.message,
        conversation_id: update.conversation_id.or(conversation_id.clone()),
        updated_at,
        artifacts: vec![],
    }))
}

pub(super) fn maybe_write_research_output(
    command: &ResearchCommand,
    job_id: &str,
    report_text: &str,
) -> Result<Option<String>> {
    let Some(output_dir) = command.output_dir.as_deref() else {
        return Ok(None);
    };

    let output_dir = PathBuf::from(output_dir);
    fs::create_dir_all(&output_dir).with_context(|| format!("create {}", output_dir.display()))?;
    let destination = unique_destination_path(
        &output_dir,
        Path::new(&format!("{}-report.md", sanitize_file_stem(job_id))),
    );
    fs::write(&destination, report_text)
        .with_context(|| format!("write {}", destination.display()))?;
    Ok(Some(destination.display().to_string()))
}

fn sanitize_file_stem(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

pub(super) fn unique_destination_path(destination_dir: &Path, file_name: &Path) -> PathBuf {
    let candidate = destination_dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let stem = file_name
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("artifact");
    let extension = file_name.extension().and_then(|value| value.to_str());

    for index in 2.. {
        let name = match extension {
            Some(extension) => format!("{stem}-{index}.{extension}"),
            None => format!("{stem}-{index}"),
        };
        let candidate = destination_dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("destination path generation should always return");
}

#[cfg(test)]
mod tests {
    use super::{
        browser_session_identity, current_url_requires_login_or_challenge, recovery_guidance,
        resolve_conversation_target_url,
    };
    use crate::{AppConfig, Database, core::CoreService};
    use chrono::Utc;
    use shared_protocol::{
        AskCommand, AuthSource, BrowserConnectionMode, BrowserOwner, BrowserStatusSnapshot, Mode,
        ResearchCommand,
    };

    fn seed_conversation(config: &AppConfig, conversation_id: &str) {
        let db = Database::open(&config.paths.db_path).unwrap();
        let now = Utc::now();
        db.upsert_session(
            "session-1",
            "Browser session",
            Some("https://chatgpt.com/c/seed"),
            now,
        )
        .unwrap();
        db.upsert_conversation(
            conversation_id,
            "session-1",
            "Browser conversation",
            Some(&format!("https://chatgpt.com/c/{conversation_id}")),
            Some("seed prompt"),
            Some("seed response"),
            now,
        )
        .unwrap();
    }

    #[test]
    fn attach_session_identity_uses_target_tab_id() {
        let snapshot = BrowserStatusSnapshot {
            running: true,
            browser_kind: "chrome".to_owned(),
            profile_dir: "profile".to_owned(),
            attach_profile_dir: None,
            attach_profile_note: None,
            downloads_dir: "downloads".to_owned(),
            target_url: "https://chatgpt.com/".to_owned(),
            current_url: Some("https://chatgpt.com/c/conv-1".to_owned()),
            page_title: None,
            page_ready: true,
            authenticated: true,
            reload_count: 0,
            screenshot_path: None,
            browser_path: None,
            extension_connected: true,
            capability_report: None,
            attach_mode: BrowserConnectionMode::AttachFirst,
            browser_owner: BrowserOwner::External,
            cdp_attached: true,
            target_tab_found: true,
            target_tab_id: Some("abcdef1234567890".to_owned()),
            target_workspace_key: Some("https://chatgpt.com/".to_owned()),
            debug_endpoint: Some("http://127.0.0.1:9222".to_owned()),
            auth_source: AuthSource::Page,
            extension_required: true,
            last_capability_report_at: None,
        };

        let (session_id, label) = browser_session_identity(&snapshot);

        assert_eq!(session_id, "browser-tab-abcdef1234567890");
        assert_eq!(label, "Browser tab abcdef12");
    }

    #[test]
    fn attach_recovery_guidance_prioritizes_endpoint_extension_auth_tab_and_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        let browser = BrowserStatusSnapshot {
            running: false,
            browser_kind: "chrome".to_owned(),
            profile_dir: config.paths.browser_profile_dir.display().to_string(),
            attach_profile_dir: None,
            attach_profile_note: None,
            downloads_dir: config.paths.downloads_dir.display().to_string(),
            target_url: config.browser_target_url().to_owned(),
            current_url: Some("https://chatgpt.com/auth/login".to_owned()),
            page_title: None,
            page_ready: false,
            authenticated: false,
            reload_count: 0,
            screenshot_path: None,
            browser_path: config.browser_path().map(ToOwned::to_owned),
            extension_connected: false,
            capability_report: None,
            attach_mode: BrowserConnectionMode::AttachFirst,
            browser_owner: BrowserOwner::External,
            cdp_attached: false,
            target_tab_found: false,
            target_tab_id: None,
            target_workspace_key: None,
            debug_endpoint: Some(config.browser_debug_endpoint()),
            auth_source: AuthSource::Unknown,
            extension_required: true,
            last_capability_report_at: None,
        };

        let guidance = recovery_guidance(&config, &browser, None);

        assert!(guidance.iter().any(|line| line.contains("attach endpoint")));
        assert!(guidance.iter().any(|line| line.contains("extension")));
        assert!(guidance.iter().any(|line| line.contains("challenge page")));
        assert!(guidance.iter().any(|line| line.contains("target tab")));
        assert!(guidance.iter().any(|line| line.contains("managed_only")));
    }

    #[test]
    fn current_url_requires_login_or_challenge_detects_cloudflare_and_login_pages() {
        assert!(current_url_requires_login_or_challenge(Some(
            "https://chatgpt.com/auth/login"
        )));
        assert!(current_url_requires_login_or_challenge(Some(
            "https://chatgpt.com/?cf_chl_rt_tk=challenge"
        )));
        assert!(!current_url_requires_login_or_challenge(Some(
            "https://chatgpt.com/c/123"
        )));
    }

    #[test]
    fn resolve_conversation_target_url_preserves_workspace_path_and_query() {
        let url = resolve_conversation_target_url(
            "https://chatgpt.com/g/acme-team?model=gpt-5",
            "conv-123",
        );
        assert_eq!(
            url,
            "https://chatgpt.com/g/acme-team/c/conv-123?model=gpt-5"
        );

        let existing = resolve_conversation_target_url(
            "https://chatgpt.com/g/acme-team/c/conv-old?model=gpt-5",
            "conv-456",
        );
        assert_eq!(
            existing,
            "https://chatgpt.com/g/acme-team/c/conv-456?model=gpt-5"
        );
    }

    #[test]
    fn prepare_ask_does_not_reuse_latest_conversation_without_explicit_request() {
        let dir = tempfile::tempdir().unwrap();
        let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        seed_conversation(&config, "conv-explicit");
        let service = CoreService::new(config).unwrap();

        let prepared = service
            .prepare_ask(AskCommand {
                prompt: "hello".to_owned(),
                mode: Mode::Normal,
                model_class: None,
                effort: None,
                files: vec![],
                conversation_id: None,
                new_conversation: false,
            })
            .unwrap();

        assert_eq!(prepared.command.conversation_id, None);
        assert!(!prepared.command.new_conversation);
    }

    #[test]
    fn prepare_ask_preserves_new_conversation_requests() {
        let dir = tempfile::tempdir().unwrap();
        let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        seed_conversation(&config, "conv-explicit");
        let service = CoreService::new(config).unwrap();

        let prepared = service
            .prepare_ask(AskCommand {
                prompt: "hello again".to_owned(),
                mode: Mode::Normal,
                model_class: None,
                effort: None,
                files: vec![],
                conversation_id: None,
                new_conversation: true,
            })
            .unwrap();

        assert_eq!(prepared.command.conversation_id, None);
        assert!(prepared.command.new_conversation);
    }

    #[test]
    fn prepare_research_does_not_reuse_latest_conversation_without_explicit_request() {
        let dir = tempfile::tempdir().unwrap();
        let config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        seed_conversation(&config, "conv-explicit");
        let service = CoreService::new(config).unwrap();

        let prepared = service
            .prepare_research(ResearchCommand {
                query: "investigate".to_owned(),
                conversation_id: None,
                files: vec![],
                output_dir: None,
                new_conversation: false,
            })
            .unwrap();

        assert_eq!(prepared.command.conversation_id, None);
        assert!(!prepared.command.new_conversation);
    }
}
