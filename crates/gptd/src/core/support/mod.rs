#![allow(unused_imports)]
use super::{
    CoreService, ExecutionRecoveryState, recovery::is_provider_generation_failure_message,
};
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

mod browser;
mod conversation;
mod errors;
mod locks;
mod request;
mod research;

pub(super) use browser::{
    busy_health_snapshot, current_url_requires_login_or_challenge, default_selector_pack_summary,
    health_snapshot, is_attach_primary_mode, recovery_guidance, supports_managed_profile_reset,
};
pub(super) use conversation::{
    browser_session_identity, conversation_id_from_url, resolve_conversation_target_url,
};
pub use errors::error_code_for_message;
pub(super) use errors::is_cancelled_message;
pub(super) use locks::{
    browser_busy_message, lock_browser, lock_cancel_registry, try_lock_browser,
};
pub(super) use research::{
    ResearchUpdateContext, ResearchUpdateProgress, apply_research_update,
    maybe_write_research_output, unique_destination_path,
};

#[cfg(test)]
mod tests {
    use super::{
        browser_session_identity, current_url_requires_login_or_challenge, error_code_for_message,
        recovery_guidance, resolve_conversation_target_url,
    };
    use crate::{AppConfig, Database, core::CoreService};
    use chrono::Utc;
    use shared_protocol::{
        AskCommand, AuthSource, BrowserConnectionMode, BrowserOwner, BrowserStatusSnapshot,
        ErrorCode, Mode, ResearchCommand,
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
}
