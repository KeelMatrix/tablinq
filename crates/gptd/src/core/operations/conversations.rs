#![allow(unused_imports)]
use super::super::CoreService;
use super::super::support::{
    browser_busy_message, browser_session_identity, default_selector_pack_summary, health_snapshot,
    is_attach_primary_mode, recovery_guidance, resolve_conversation_target_url, try_lock_browser,
};
use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use shared_protocol::{
    BrowserDoctor, BrowserOpenRequest, BrowserStatusSnapshot, CapabilitySnapshot, ConfigSnapshot,
    ConversationList, ConversationSelectCommand, ConversationSelection, ConversationSummary,
    DoctorReport, SessionConversationList, SessionList,
};

impl CoreService {
    pub fn list_sessions(&self) -> Result<SessionList> {
        self.open_db()?.list_sessions()
    }

    pub fn find_conversation(&self, conversation_id: &str) -> Result<Option<ConversationSummary>> {
        self.open_db()?.find_conversation(conversation_id)
    }

    pub fn list_conversations(&self) -> Result<ConversationList> {
        self.open_db()?.list_conversations()
    }

    pub fn list_session_conversations(
        &self,
        session_id: &str,
    ) -> Result<Option<SessionConversationList>> {
        let db = self.open_db()?;
        let Some(session) = db.find_session_info(session_id)? else {
            return Ok(None);
        };
        Ok(Some(SessionConversationList {
            session,
            conversations: db.list_session_conversations(session_id)?,
        }))
    }

    pub fn select_conversation(
        &self,
        command: ConversationSelectCommand,
    ) -> Result<ConversationSelection> {
        command
            .validate()
            .context("validate conversation select command")?;

        let target_conversation = if let Some(conversation_id) = command.conversation_id.as_deref()
        {
            Some(
                self.open_db()?
                    .find_conversation(conversation_id)?
                    .with_context(|| format!("conversation {conversation_id} was not found"))?,
            )
        } else {
            None
        };

        let target_url = if let Some(conversation) = target_conversation.as_ref() {
            conversation
                .tab_url
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| {
                    resolve_conversation_target_url(
                        self.config.browser_target_url(),
                        &conversation.conversation_id,
                    )
                })
        } else {
            self.config.browser_target_url().to_owned()
        };

        let snapshot = if command.new_conversation {
            let mut browser =
                try_lock_browser(&self.browser)?.ok_or_else(|| anyhow!(browser_busy_message()))?;
            let previous_url = browser.health()?.current_url;
            let previous_conversation_id = previous_url
                .as_deref()
                .and_then(super::super::support::conversation_id_from_url);
            let snapshot = browser.open_fresh_conversation(BrowserOpenRequest {
                target_url: Some(target_url),
                browser_path: None,
                headless: None,
            })?;
            if previous_conversation_id.is_some()
                && snapshot.current_url.as_deref() == previous_url.as_deref()
            {
                anyhow::bail!(
                    "fresh conversation navigation did not leave the previous provider thread"
                );
            }
            if let Some(previous_conversation_id) = previous_conversation_id
                && snapshot
                    .current_url
                    .as_deref()
                    .and_then(super::super::support::conversation_id_from_url)
                    .as_deref()
                    == Some(previous_conversation_id.as_str())
            {
                anyhow::bail!(
                    "fresh conversation navigation is still on the previous provider thread"
                );
            }
            snapshot
        } else {
            self.browser_open(BrowserOpenRequest {
                target_url: Some(target_url),
                browser_path: None,
                headless: None,
            })?
        };
        let (session_id, session_label) = browser_session_identity(&snapshot);
        self.open_db()?.upsert_session(
            &session_id,
            &session_label,
            snapshot.current_url.as_deref(),
            Utc::now(),
        )?;

        Ok(ConversationSelection {
            conversation_id: target_conversation.map(|conversation| conversation.conversation_id),
            session_id: Some(session_id),
            current_url: snapshot.current_url.clone(),
            browser: snapshot,
        })
    }
}
