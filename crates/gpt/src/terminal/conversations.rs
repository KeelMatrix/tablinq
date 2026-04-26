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
    let mut output = String::from("New conversation ready");
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
    use shared_protocol::ConversationSummary;

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
