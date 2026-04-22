use super::{CoreService, ExecutionRecoveryState};
use crate::browser::{
    BrowserDiagnosticsSnapshot, BrowserManager, RequestLifecycleUpdate, ResearchUpdate,
};
use crate::config::AppConfig;
use crate::db::Database;
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use shared_protocol::{
    ArtifactMetadata, BrowserConnectionMode, BrowserStatusSnapshot, ConversationRecoveryManifest,
    ConversationSummary, Effort, ErrorCode, EventPayload, HealthCheck, HealthStatus, JobState,
    JobStatus, Mode, ModelClass, RecoveryManifestFile, RecoveryManifestFileKind,
    RecoveryManifestTurn, RecoveryPacket, RecoveryStrategy, RecoveryTrace, ResearchCommand,
    SelectorPackSummary, StreamChunk, StreamChunkKind,
};
use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use url::Url;

const PROVIDER_GENERATION_FAILURE_MARKER: &str =
    "something went wrong while generating the response";
const RECOVERY_MANIFEST_RECENT_TURNS_LIMIT: usize = 6;
const RECOVERY_MANIFEST_ARCHIVED_CONTEXT_LIMIT: usize = 8;
const RECOVERY_MANIFEST_RESPONSE_EXCERPT_LIMIT: usize = 720;
const RECOVERY_CONTEXT_SUMMARY_RECENT_TURNS: usize = 3;
const RECOVERY_REPLAY_FILE_LIMIT: usize = 10;
const RECOVERY_REPLAY_HISTORICAL_FILE_LIMIT: usize = 4;

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
    let message_lower = message.to_ascii_lowercase();
    if message_lower.contains(PROVIDER_GENERATION_FAILURE_MARKER) {
        ErrorCode::ProviderGenerationFailedNonretryable
    } else if message.contains("profile locked") || message.contains("SingletonLock") {
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

pub(super) fn should_auto_recover_provider_generation_failure(
    auto_recover: bool,
    recovery_attempt_count: u32,
) -> bool {
    auto_recover && recovery_attempt_count < 1
}

pub(super) fn provider_recovery_attempt_request_id(job_id: &str, attempt: u32) -> String {
    format!("{job_id}:provider-attempt-{attempt}")
}

pub(super) fn build_provider_generation_recovery_packet(
    original_input: &str,
    mode: Mode,
    model_class: Option<ModelClass>,
    effort: Option<Effort>,
    files: &[String],
    context_summary: Option<String>,
    context_manifest: Option<ConversationRecoveryManifest>,
    recovery_manifest_path: Option<String>,
    replay_files: Vec<String>,
    prior_request_id: Option<String>,
    prior_conversation_id: Option<String>,
    recovery_attempt_count: u32,
) -> RecoveryPacket {
    RecoveryPacket {
        original_input: original_input.to_owned(),
        mode,
        model_class,
        effort,
        files: files.to_vec(),
        context_summary,
        context_manifest,
        recovery_manifest_path,
        replay_files,
        prior_request_id,
        prior_conversation_id,
        failure_code: ErrorCode::ProviderGenerationFailedNonretryable,
        recovery_attempt_count,
    }
}

pub(super) fn build_provider_generation_recovery_trace(
    failure_bundle_id: Option<String>,
    prior_request_id: Option<String>,
    prior_conversation_id: Option<String>,
    recovery_attempt_count: u32,
    recovery_attempted: bool,
) -> RecoveryTrace {
    RecoveryTrace {
        recovery_available: true,
        recovery_attempted,
        recovery_strategy: Some(RecoveryStrategy::ReloadNewConversationReplayWithContext),
        failure_code: Some(ErrorCode::ProviderGenerationFailedNonretryable),
        failure_bundle_id,
        prior_request_id,
        replacement_request_id: None,
        prior_conversation_id,
        replacement_conversation_id: None,
        recovery_attempt_count,
    }
}

pub(super) fn build_provider_generation_context_summary(
    manifest: Option<&ConversationRecoveryManifest>,
    conversation: Option<&ConversationSummary>,
    prior_conversation_id: Option<&str>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(conversation_id) = prior_conversation_id
        && !conversation_id.trim().is_empty()
    {
        parts.push(format!("Prior conversation id: {}", conversation_id.trim()));
    }
    if let Some(manifest) = manifest {
        if !manifest.archived_context.is_empty() {
            parts.push("Earlier preserved context:".to_owned());
            for line in manifest.archived_context.iter().rev().take(2).rev() {
                parts.push(format!("- {}", truncate_recovery_text(line, 220)));
            }
        }

        let recent_turns = manifest
            .recent_turns
            .iter()
            .rev()
            .take(RECOVERY_CONTEXT_SUMMARY_RECENT_TURNS)
            .collect::<Vec<_>>();
        for (index, turn) in recent_turns.into_iter().rev().enumerate() {
            let turn_number = index + 1;
            parts.push(format!(
                "Recent request {turn_number}: {}",
                truncate_recovery_text(&turn.request_text, 220)
            ));
            parts.push(format!(
                "Recent assistant result {turn_number}: {}",
                truncate_recovery_text(&turn.response_excerpt, 320)
            ));
            if !turn.input_files.is_empty() {
                parts.push(format!(
                    "Recent input files {turn_number}: {}",
                    turn.input_files.join(", ")
                ));
            }
            if !turn.generated_artifacts.is_empty() {
                parts.push(format!(
                    "Recent generated artifacts {turn_number}: {}",
                    turn.generated_artifacts.join(", ")
                ));
            }
        }
    } else if let Some(conversation) = conversation {
        if let Some(last_prompt) = conversation.last_prompt.as_deref()
            && !last_prompt.trim().is_empty()
        {
            parts.push(format!(
                "Previous prompt: {}",
                truncate_recovery_text(last_prompt, 240)
            ));
        }
        if let Some(last_response) = conversation.last_response.as_deref()
            && !last_response.trim().is_empty()
        {
            parts.push(format!(
                "Previous assistant response: {}",
                truncate_recovery_text(last_response, 360)
            ));
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

pub(super) fn build_provider_generation_replay_prompt(
    original_input: &str,
    context_summary: Option<&str>,
    recovery_manifest_attachment: Option<&str>,
) -> String {
    let mut prompt = String::from(
        "Recovery note: The previous provider generation failed. Continue this request in a fresh conversation using the preserved context below.",
    );
    if let Some(file_name) = recovery_manifest_attachment
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        prompt.push_str("\n\nA recovery manifest attachment is included: ");
        prompt.push_str(file_name);
        prompt.push_str(
            ". Use it as the durable source for preserved recent context, tracked files, and generated artifacts.",
        );
    }
    if let Some(summary) = context_summary
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        prompt.push_str("\n\nPreserved context:\n");
        prompt.push_str(summary);
    }
    prompt.push_str("\n\nOriginal user request:\n");
    prompt.push_str(original_input.trim());
    prompt
}

pub(super) fn update_conversation_recovery_manifest(
    existing: Option<&ConversationRecoveryManifest>,
    conversation_id: &str,
    request_text: &str,
    response_text: &str,
    files: &[String],
    artifacts: &[ArtifactMetadata],
    completed_at: DateTime<Utc>,
) -> ConversationRecoveryManifest {
    let mut manifest = existing
        .cloned()
        .unwrap_or_else(|| ConversationRecoveryManifest {
            conversation_id: conversation_id.to_owned(),
            archived_context: Vec::new(),
            recent_turns: Vec::new(),
            tracked_input_files: Vec::new(),
            tracked_generated_artifacts: Vec::new(),
            updated_at: completed_at,
        });
    manifest.conversation_id = conversation_id.to_owned();
    manifest.updated_at = completed_at;

    for file in files {
        remember_recovery_manifest_file(
            &mut manifest.tracked_input_files,
            RecoveryManifestFile {
                kind: RecoveryManifestFileKind::InputAttachment,
                file_name: recovery_file_name(file),
                local_path: file.to_owned(),
                content_type: None,
                recorded_at: completed_at,
            },
        );
    }
    for artifact in artifacts {
        remember_recovery_manifest_file(
            &mut manifest.tracked_generated_artifacts,
            RecoveryManifestFile {
                kind: RecoveryManifestFileKind::GeneratedArtifact,
                file_name: artifact.file_name.clone(),
                local_path: artifact.storage_path.clone(),
                content_type: Some(artifact.content_type.clone()),
                recorded_at: completed_at,
            },
        );
    }

    manifest.recent_turns.push(RecoveryManifestTurn {
        completed_at,
        request_text: request_text.trim().to_owned(),
        response_excerpt: truncate_recovery_text(
            response_text,
            RECOVERY_MANIFEST_RESPONSE_EXCERPT_LIMIT,
        ),
        input_files: files.iter().map(|file| recovery_file_name(file)).collect(),
        generated_artifacts: artifacts
            .iter()
            .map(|artifact| artifact.file_name.clone())
            .collect(),
    });

    while manifest.recent_turns.len() > RECOVERY_MANIFEST_RECENT_TURNS_LIMIT {
        let archived = manifest.recent_turns.remove(0);
        manifest
            .archived_context
            .push(summarize_recovery_manifest_turn(&archived));
    }
    while manifest.archived_context.len() > RECOVERY_MANIFEST_ARCHIVED_CONTEXT_LIMIT {
        manifest.archived_context.remove(0);
    }

    manifest
}

pub(super) fn build_provider_generation_replay_files(
    files: &[String],
    manifest: Option<&ConversationRecoveryManifest>,
    recovery_manifest_path: Option<&str>,
) -> Vec<String> {
    let mut replay_files = Vec::new();
    let mut seen = HashSet::new();
    let mut historical_added = 0usize;

    for file in files {
        push_replay_file(&mut replay_files, &mut seen, file);
    }

    if let Some(path) = recovery_manifest_path {
        push_replay_file(&mut replay_files, &mut seen, path);
    }

    if let Some(manifest) = manifest {
        for file in &manifest.tracked_generated_artifacts {
            if historical_added >= RECOVERY_REPLAY_HISTORICAL_FILE_LIMIT {
                break;
            }
            if !Path::new(&file.local_path).exists() {
                continue;
            }
            let before = replay_files.len();
            push_replay_file(&mut replay_files, &mut seen, &file.local_path);
            if replay_files.len() > before {
                historical_added += 1;
            }
        }
        for file in &manifest.tracked_input_files {
            if historical_added >= RECOVERY_REPLAY_HISTORICAL_FILE_LIMIT {
                break;
            }
            if !Path::new(&file.local_path).exists() {
                continue;
            }
            let before = replay_files.len();
            push_replay_file(&mut replay_files, &mut seen, &file.local_path);
            if replay_files.len() > before {
                historical_added += 1;
            }
        }
    }

    replay_files
}

pub(super) fn render_provider_generation_recovery_manifest(
    manifest: &ConversationRecoveryManifest,
) -> String {
    let mut content = String::from("# Tablinq Recovery Manifest\n\n");
    content.push_str(
        "This attachment preserves compact durable context for replay after a provider generation failure.\n\n",
    );
    content.push_str(&format!(
        "- Conversation ID: {}\n- Updated at: {}\n\n",
        manifest.conversation_id,
        manifest.updated_at.to_rfc3339()
    ));

    if !manifest.archived_context.is_empty() {
        content.push_str("## Earlier Context\n");
        for line in &manifest.archived_context {
            content.push_str("- ");
            content.push_str(line);
            content.push('\n');
        }
        content.push('\n');
    }

    if !manifest.recent_turns.is_empty() {
        content.push_str("## Recent Turns\n");
        for (index, turn) in manifest.recent_turns.iter().enumerate() {
            content.push_str(&format!(
                "### Turn {}\n- Completed at: {}\n- User request: {}\n- Assistant result: {}\n",
                index + 1,
                turn.completed_at.to_rfc3339(),
                turn.request_text,
                turn.response_excerpt
            ));
            if !turn.input_files.is_empty() {
                content.push_str("- Input files: ");
                content.push_str(&turn.input_files.join(", "));
                content.push('\n');
            }
            if !turn.generated_artifacts.is_empty() {
                content.push_str("- Generated artifacts: ");
                content.push_str(&turn.generated_artifacts.join(", "));
                content.push('\n');
            }
        }
        content.push('\n');
    }

    if !manifest.tracked_input_files.is_empty() {
        content.push_str("## Tracked Input Files\n");
        for file in &manifest.tracked_input_files {
            content.push_str(&format!("- {} ({})\n", file.file_name, file.local_path));
        }
        content.push('\n');
    }

    if !manifest.tracked_generated_artifacts.is_empty() {
        content.push_str("## Tracked Generated Artifacts\n");
        for file in &manifest.tracked_generated_artifacts {
            content.push_str(&format!(
                "- {} ({}){}\n",
                file.file_name,
                file.local_path,
                file.content_type
                    .as_deref()
                    .map(|value| format!(" [{}]", value))
                    .unwrap_or_default()
            ));
        }
    }

    content
}

pub(super) fn write_provider_generation_recovery_manifest_attachment(
    state_root: &Path,
    job_id: &str,
    attempt: u32,
    manifest: &ConversationRecoveryManifest,
) -> Result<String> {
    let dir = state_root.join("recovery-manifests");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let safe_job_id = job_id.replace([':', '/', '\\'], "-");
    let path = dir.join(format!(
        "{safe_job_id}-provider-recovery-attempt-{attempt}.md"
    ));
    fs::write(
        &path,
        render_provider_generation_recovery_manifest(manifest),
    )
    .with_context(|| format!("write {}", path.display()))?;
    Ok(path.display().to_string())
}

fn truncate_recovery_text(text: &str, limit: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_owned();
    }

    let mut output = String::new();
    for ch in trimmed.chars().take(limit.saturating_sub(1)) {
        output.push(ch);
    }
    output.push('…');
    output
}

fn remember_recovery_manifest_file(
    files: &mut Vec<RecoveryManifestFile>,
    entry: RecoveryManifestFile,
) {
    files.retain(|candidate| candidate.local_path != entry.local_path);
    files.insert(0, entry);
}

fn push_replay_file(files: &mut Vec<String>, seen: &mut HashSet<String>, path: &str) {
    let trimmed = path.trim();
    if trimmed.is_empty() || files.len() >= RECOVERY_REPLAY_FILE_LIMIT {
        return;
    }
    if !seen.insert(trimmed.to_owned()) {
        return;
    }
    files.push(trimmed.to_owned());
}

fn summarize_recovery_manifest_turn(turn: &RecoveryManifestTurn) -> String {
    let mut parts = vec![
        format!(
            "User request: {}",
            truncate_recovery_text(&turn.request_text, 160)
        ),
        format!(
            "Assistant result: {}",
            truncate_recovery_text(&turn.response_excerpt, 220)
        ),
    ];
    if !turn.input_files.is_empty() {
        parts.push(format!("Input files: {}", turn.input_files.join(", ")));
    }
    if !turn.generated_artifacts.is_empty() {
        parts.push(format!(
            "Generated artifacts: {}",
            turn.generated_artifacts.join(", ")
        ));
    }
    parts.join(" | ")
}

fn recovery_file_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path.to_owned())
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

pub(super) fn conversation_id_from_url(raw_url: &str) -> Option<String> {
    let marker = "/c/";
    let start = raw_url.find(marker)? + marker.len();
    let rest = &raw_url[start..];
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let conversation_id = rest[..end].trim();
    if conversation_id.is_empty() {
        None
    } else {
        Some(conversation_id.to_owned())
    }
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
        browser_session_identity, build_provider_generation_context_summary,
        build_provider_generation_replay_files, build_provider_generation_replay_prompt,
        current_url_requires_login_or_challenge, error_code_for_message, recovery_guidance,
        resolve_conversation_target_url, should_auto_recover_provider_generation_failure,
        update_conversation_recovery_manifest,
    };
    use crate::{AppConfig, Database, core::CoreService};
    use chrono::Utc;
    use shared_protocol::{
        ArtifactMetadata, AskCommand, AuthSource, BrowserConnectionMode, BrowserOwner,
        BrowserStatusSnapshot, ConversationSummary, ErrorCode, Mode, ResearchCommand,
    };
    use tempfile::tempdir;

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
            None,
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
                auto_recover_provider_generation_failure: true,
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
                auto_recover_provider_generation_failure: true,
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

    #[test]
    fn error_code_for_message_classifies_provider_generation_failure() {
        assert_eq!(
            error_code_for_message(
                "Something went wrong while generating the response. If this issue persists please contact us through our help center."
            ),
            ErrorCode::ProviderGenerationFailedNonretryable
        );
    }

    #[test]
    fn provider_generation_context_summary_uses_prior_conversation_snapshot() {
        let summary = build_provider_generation_context_summary(
            None,
            Some(&ConversationSummary {
                conversation_id: "conv-123".to_owned(),
                session_id: "session-1".to_owned(),
                label: "Browser conversation".to_owned(),
                tab_url: Some("https://chatgpt.com/c/conv-123".to_owned()),
                last_prompt: Some("Explain the current system state in detail".to_owned()),
                last_response: Some("The current system state includes attached files and a partially completed workflow.".to_owned()),
                last_seen_at: Utc::now(),
            }),
            Some("conv-123"),
        )
        .unwrap();

        assert!(summary.contains("Prior conversation id: conv-123"));
        assert!(summary.contains("Previous prompt: Explain the current system state in detail"));
        assert!(summary.contains(
            "Previous assistant response: The current system state includes attached files"
        ));
    }

    #[test]
    fn provider_generation_replay_prompt_wraps_context_and_original_request() {
        let prompt = build_provider_generation_replay_prompt(
            "reason carefully about this tradeoff",
            Some("Prior conversation id: conv-123\nPrevious assistant response: partial answer"),
            Some("job-1-provider-recovery.md"),
        );

        assert!(prompt.contains("Recovery note: The previous provider generation failed."));
        assert!(prompt.contains("A recovery manifest attachment is included"));
        assert!(prompt.contains("Preserved context:\nPrior conversation id: conv-123"));
        assert!(prompt.ends_with("Original user request:\nreason carefully about this tradeoff"));
    }

    #[test]
    fn provider_generation_recovery_policy_attempts_once() {
        assert!(should_auto_recover_provider_generation_failure(true, 0));
        assert!(!should_auto_recover_provider_generation_failure(true, 1));
        assert!(!should_auto_recover_provider_generation_failure(false, 0));
    }

    #[test]
    fn conversation_recovery_manifest_tracks_recent_turns_and_files() {
        let dir = tempdir().unwrap();
        let input_path = dir.path().join("brief.txt");
        let artifact_path = dir.path().join("summary.txt");
        std::fs::write(&input_path, "brief").unwrap();
        std::fs::write(&artifact_path, "summary").unwrap();

        let manifest = update_conversation_recovery_manifest(
            None,
            "conv-1",
            "Summarize the brief",
            "The brief recommends a phased rollout with one generated summary.",
            &[input_path.display().to_string()],
            &[ArtifactMetadata {
                artifact_id: "artifact-1".to_owned(),
                conversation_id: Some("conv-1".to_owned()),
                message_id: Some("msg-1".to_owned()),
                file_name: "summary.txt".to_owned(),
                content_type: "text/plain".to_owned(),
                size_bytes: 7,
                sha256: "sha-1".to_owned(),
                storage_path: artifact_path.display().to_string(),
            }],
            Utc::now(),
        );

        assert_eq!(manifest.recent_turns.len(), 1);
        assert_eq!(
            manifest.recent_turns[0].input_files,
            vec!["brief.txt".to_owned()]
        );
        assert_eq!(
            manifest.recent_turns[0].generated_artifacts,
            vec!["summary.txt".to_owned()]
        );
        assert_eq!(manifest.tracked_input_files.len(), 1);
        assert_eq!(manifest.tracked_generated_artifacts.len(), 1);
    }

    #[test]
    fn provider_generation_context_summary_prefers_durable_manifest() {
        let manifest = update_conversation_recovery_manifest(
            None,
            "conv-123",
            "Explain the deployment history",
            "The deployment history includes an earlier rollback and a generated runbook artifact.",
            &["C:\\inputs\\deploy-notes.md".to_owned()],
            &[],
            Utc::now(),
        );

        let summary = build_provider_generation_context_summary(
            Some(&manifest),
            Some(&ConversationSummary {
                conversation_id: "conv-123".to_owned(),
                session_id: "session-1".to_owned(),
                label: "Browser conversation".to_owned(),
                tab_url: Some("https://chatgpt.com/c/conv-123".to_owned()),
                last_prompt: Some("fallback prompt".to_owned()),
                last_response: Some("fallback response".to_owned()),
                last_seen_at: Utc::now(),
            }),
            Some("conv-123"),
        )
        .unwrap();

        assert!(summary.contains("Recent request 1: Explain the deployment history"));
        assert!(summary.contains("Recent input files 1: deploy-notes.md"));
        assert!(!summary.contains("fallback prompt"));
    }

    #[test]
    fn provider_generation_replay_files_include_manifest_and_historical_context() {
        let dir = tempdir().unwrap();
        let original_input = dir.path().join("latest.txt");
        let prior_input = dir.path().join("brief.txt");
        let artifact = dir.path().join("artifact.txt");
        let manifest_file = dir.path().join("tablinq-recovery.md");
        std::fs::write(&original_input, "latest").unwrap();
        std::fs::write(&prior_input, "brief").unwrap();
        std::fs::write(&artifact, "artifact").unwrap();
        std::fs::write(&manifest_file, "manifest").unwrap();

        let manifest = update_conversation_recovery_manifest(
            None,
            "conv-1",
            "Use the brief",
            "Generated artifact attached.",
            &[prior_input.display().to_string()],
            &[ArtifactMetadata {
                artifact_id: "artifact-1".to_owned(),
                conversation_id: Some("conv-1".to_owned()),
                message_id: Some("msg-1".to_owned()),
                file_name: "artifact.txt".to_owned(),
                content_type: "text/plain".to_owned(),
                size_bytes: 8,
                sha256: "sha-1".to_owned(),
                storage_path: artifact.display().to_string(),
            }],
            Utc::now(),
        );

        let replay_files = build_provider_generation_replay_files(
            &[original_input.display().to_string()],
            Some(&manifest),
            Some(&manifest_file.display().to_string()),
        );

        assert_eq!(replay_files[0], original_input.display().to_string());
        assert!(replay_files.contains(&manifest_file.display().to_string()));
        assert!(replay_files.contains(&artifact.display().to_string()));
        assert!(replay_files.contains(&prior_input.display().to_string()));
    }
}
