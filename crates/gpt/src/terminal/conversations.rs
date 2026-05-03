use shared_protocol::{ConversationList, ConversationSelection};

const PROMPT_LIMIT: usize = 80;

pub(crate) fn format_conversation_list(
    list: &ConversationList,
    active_conversation_id: Option<&str>,
) -> String {
    if list.conversations.is_empty() {
        return "No conversations found".to_owned();
    }

    let mut output = String::from("Conversations\n");
    for conversation in &list.conversations {
        output.push('\n');
        let marker = if active_conversation_id == Some(conversation.conversation_id.as_str()) {
            "*"
        } else {
            " "
        };
        output.push_str(&format!(
            "{} {}  {}  {}  {}\n",
            marker,
            short_id(&conversation.conversation_id),
            conversation.last_seen_at.format("%Y-%m-%d %H:%M"),
            short_id(&conversation.session_id),
            conversation.label
        ));
        if let Some(prompt) = conversation
            .last_prompt
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            output.push_str(&format!("  prompt: {}\n", truncate(prompt, PROMPT_LIMIT)));
        }
        if let Some(url) = conversation
            .tab_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            output.push_str(&format!("  url: {url}\n"));
        }
    }

    output.trim_end().to_owned()
}

pub(crate) fn format_conversation_selection(selection: &ConversationSelection) -> String {
    let mut output = if selection.conversation_id.is_some() {
        String::from("Conversation selected")
    } else {
        String::from("New conversation ready")
    };
    if let Some(conversation_id) = selection.conversation_id.as_deref() {
        output.push_str(&format!("\nconversation: {}", short_id(conversation_id)));
    }
    if let Some(session_id) = selection.session_id.as_deref() {
        output.push_str(&format!("\nsession: {}", short_id(session_id)));
    }
    if let Some(url) = selection.current_url.as_deref() {
        output.push_str(&format!("\nurl: {url}"));
    }
    output
}

fn short_id(value: &str) -> String {
    truncate(value, 8)
}

fn truncate(value: &str, limit: usize) -> String {
    let mut chars = value.chars();
    let mut output: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        output.push_str("...");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use shared_protocol::{AuthSource, BrowserOwner, BrowserStatusSnapshot, ConversationSummary};

    fn conversation(id: &str, prompt: Option<&str>, response: Option<&str>) -> ConversationSummary {
        ConversationSummary {
            conversation_id: id.to_owned(),
            session_id: "session-123456789".to_owned(),
            label: "Browser conversation".to_owned(),
            tab_url: Some(format!("https://chatgpt.com/c/{id}")),
            last_prompt: prompt.map(str::to_owned),
            last_response: response.map(str::to_owned),
            last_seen_at: Utc.with_ymd_and_hms(2026, 4, 26, 13, 17, 0).unwrap(),
        }
    }

    fn selection(conversation_id: Option<&str>) -> ConversationSelection {
        ConversationSelection {
            conversation_id: conversation_id.map(str::to_owned),
            session_id: Some("session-123456789".to_owned()),
            current_url: Some("https://chatgpt.com/c/conv-selected".to_owned()),
            browser: BrowserStatusSnapshot {
                running: true,
                browser_kind: "chrome".to_owned(),
                profile_dir: "profile".to_owned(),
                attach_profile_dir: None,
                attach_profile_note: None,
                downloads_dir: "downloads".to_owned(),
                target_url: "https://chatgpt.com/".to_owned(),
                current_url: Some("https://chatgpt.com/c/conv-selected".to_owned()),
                page_title: Some("ChatGPT".to_owned()),
                page_ready: true,
                authenticated: true,
                reload_count: 0,
                screenshot_path: None,
                browser_path: None,
                extension_connected: true,
                capability_report: None,
                attach_mode: shared_protocol::BrowserConnectionMode::AttachFirst,
                browser_owner: BrowserOwner::External,
                cdp_attached: true,
                target_tab_found: true,
                target_tab_id: Some("tab-1".to_owned()),
                target_workspace_key: Some("https://chatgpt.com/".to_owned()),
                debug_endpoint: Some("http://127.0.0.1:9222".to_owned()),
                auth_source: AuthSource::Page,
                extension_required: true,
                last_capability_report_at: None,
                action: None,
                browser_process_closed: None,
                reason: None,
            },
        }
    }

    #[test]
    fn selection_formatter_uses_text_for_existing_conversations() {
        let output = format_conversation_selection(&selection(Some("conv-selected")));

        assert!(output.starts_with("Conversation selected"));
        assert!(output.contains("conversation: conv-sel"));
        assert!(!output.trim_start().starts_with('{'));
    }

    #[test]
    fn selection_formatter_keeps_new_conversation_label() {
        let output = format_conversation_selection(&selection(None));

        assert!(output.starts_with("New conversation ready"));
    }

    #[test]
    fn list_formatter_omits_last_response_by_default() {
        let output = format_conversation_list(
            &ConversationList {
                conversations: vec![conversation(
                    "69ee0e71-1814-8323-b4c7-0252957902cb",
                    Some("zig"),
                    Some("very long response body"),
                )],
            },
            None,
        );

        assert!(output.contains("prompt: zig"));
        assert!(!output.contains("very long response body"));
        assert!(!output.contains("last_response"));
    }

    #[test]
    fn list_formatter_truncates_long_prompts_on_char_boundaries() {
        let prompt = format!("{}{}", "λ".repeat(90), "done");
        let output = format_conversation_list(
            &ConversationList {
                conversations: vec![conversation("conv-long", Some(&prompt), None)],
            },
            None,
        );

        assert!(output.contains(&format!("prompt: {}...", "λ".repeat(PROMPT_LIMIT))));
        assert!(!output.contains("done"));
    }

    #[test]
    fn list_formatter_marks_active_conversation_when_supplied() {
        let output = format_conversation_list(
            &ConversationList {
                conversations: vec![
                    conversation("conv-active", Some("active prompt"), None),
                    conversation("conv-other", Some("other prompt"), None),
                ],
            },
            Some("conv-active"),
        );

        assert!(output.contains("* conv-act"));
        assert!(output.contains("  conv-oth"));
    }

    #[test]
    fn list_formatter_has_stable_empty_state() {
        assert_eq!(
            format_conversation_list(
                &ConversationList {
                    conversations: vec![]
                },
                None
            ),
            "No conversations found"
        );
    }
}
