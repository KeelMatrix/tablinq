#![allow(unused_imports)]
use super::super::CoreService;
use super::super::support::{
    browser_busy_message, browser_session_identity, default_selector_pack_summary, health_snapshot,
    is_attach_primary_mode, recovery_guidance, resolve_conversation_target_url,
    supports_managed_profile_reset, try_lock_browser,
};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use shared_protocol::{
    BrowserDoctor, BrowserOpenRequest, BrowserStatusSnapshot, CapabilitySnapshot, ConfigSnapshot,
    ConversationList, ConversationSelectCommand, ConversationSelection, ConversationSummary,
    DoctorReport, SessionConversationList, SessionList,
};

impl CoreService {
    pub fn browser_open(&self, request: BrowserOpenRequest) -> Result<BrowserStatusSnapshot> {
        let snapshot = try_lock_browser(&self.browser)?
            .ok_or_else(|| anyhow!(browser_busy_message()))?
            .open(request)?;
        let (session_id, session_label) = browser_session_identity(&snapshot);
        self.open_db()?.upsert_session(
            &session_id,
            &session_label,
            snapshot.current_url.as_deref(),
            Utc::now(),
        )?;
        Ok(snapshot)
    }

    pub fn browser_close(&self) -> Result<BrowserStatusSnapshot> {
        try_lock_browser(&self.browser)?
            .ok_or_else(|| anyhow!(browser_busy_message()))?
            .close()
    }

    pub fn browser_logout(&self) -> Result<BrowserStatusSnapshot> {
        let snapshot = if is_attach_primary_mode(self.config.browser_connection_mode()) {
            let mut browser =
                try_lock_browser(&self.browser)?.ok_or_else(|| anyhow!(browser_busy_message()))?;
            let current = browser.health()?;
            browser.open(BrowserOpenRequest {
                target_url: Some(current.target_url),
                browser_path: current.browser_path,
                headless: Some(false),
            })?
        } else {
            try_lock_browser(&self.browser)?
                .ok_or_else(|| anyhow!(browser_busy_message()))?
                .logout()?
        };
        let (session_id, session_label) = browser_session_identity(&snapshot);
        self.open_db()?.upsert_session(
            &session_id,
            &session_label,
            snapshot.current_url.as_deref(),
            Utc::now(),
        )?;
        Ok(snapshot)
    }

    pub fn browser_reset_profile(&self) -> Result<BrowserStatusSnapshot> {
        if !supports_managed_profile_reset(self.config.browser_connection_mode()) {
            anyhow::bail!(
                "browser reset-profile is only available for managed browser recovery; current mode is attach_first"
            );
        }
        try_lock_browser(&self.browser)?
            .ok_or_else(|| anyhow!(browser_busy_message()))?
            .reset_profile()
    }

    pub fn browser_profile_info(&self) -> Result<BrowserStatusSnapshot> {
        try_lock_browser(&self.browser)?
            .ok_or_else(|| anyhow!(browser_busy_message()))?
            .profile_info()
    }
}
