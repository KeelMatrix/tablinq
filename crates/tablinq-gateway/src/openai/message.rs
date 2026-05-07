use anyhow::Result;

use super::request::{ChatCompletionsRequest, MessageContent, invalid_request_error};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlattenedMessages {
    pub prompt: String,
    pub file_paths: Vec<String>,
    pub file_ids: Vec<String>,
}

pub fn flatten_messages(request: &ChatCompletionsRequest) -> Result<FlattenedMessages> {
    let mut sections = Vec::new();
    let mut file_paths = request.tablinq_files.clone();
    let mut file_ids = request.tablinq_file_ids.clone();

    for message in &request.messages {
        if message.role.trim() != "user" {
            return Err(invalid_request_error(
                "unsupported_role",
                format!(
                    "message role {} is not supported; only user messages are accepted for browser-backed submission",
                    message.role
                ),
            )
            .into());
        }
        let mut text_parts = Vec::new();
        match message.content.as_ref() {
            Some(MessageContent::Text(text)) => {
                if !text.trim().is_empty() {
                    text_parts.push(text.to_owned());
                }
            }
            Some(MessageContent::Parts(parts)) => {
                for part in parts {
                    match part.kind.as_str() {
                        "text" => {
                            if let Some(text) = part.text.as_ref()
                                && !text.trim().is_empty()
                            {
                                text_parts.push(text.to_owned());
                            }
                        }
                        "input_text" => {
                            if let Some(text) = part.input_text.as_ref()
                                && !text.trim().is_empty()
                            {
                                text_parts.push(text.to_owned());
                            }
                        }
                        "input_file" => {
                            if let Some(file_id) = part.file_id.as_ref() {
                                if file_id.trim().is_empty() {
                                    return Err(invalid_request_error(
                                        "invalid_file_reference",
                                        "input_file file_id must not be empty",
                                    )
                                    .into());
                                }
                                file_ids.push(file_id.trim().to_owned());
                            } else if let Some(file_path) = part.file_path.as_ref() {
                                if file_path.trim().is_empty() {
                                    return Err(invalid_request_error(
                                        "invalid_file_reference",
                                        "input_file file_path must not be empty",
                                    )
                                    .into());
                                }
                                file_paths.push(file_path.trim().to_owned());
                            } else {
                                return Err(invalid_request_error(
                                    "invalid_file_reference",
                                    "input_file parts must include file_id or file_path",
                                )
                                .into());
                            }
                        }
                        other => {
                            return Err(invalid_request_error(
                                "unsupported_content_part",
                                format!("message content part type {other} is not supported"),
                            )
                            .into());
                        }
                    }
                }
            }
            None => {
                return Err(invalid_request_error(
                    "invalid_message_content",
                    "user messages must include text content",
                )
                .into());
            }
        }

        if !text_parts.is_empty() {
            sections.push(text_parts.join("\n"));
        }
    }

    if sections.is_empty() {
        return Err(invalid_request_error(
            "invalid_message_content",
            "messages do not contain any supported text content",
        )
        .into());
    }

    Ok(FlattenedMessages {
        prompt: sections.join("\n\n"),
        file_paths,
        file_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_user_text_input_text_and_input_file_parts_without_role_prefixes() {
        let request: ChatCompletionsRequest = serde_json::from_value(serde_json::json!({
            "model": "tablinq-instant",
            "tablinq_files": ["seed.txt"],
            "tablinq_file_ids": ["file-seed"],
            "messages": [
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "first line" },
                        { "type": "input_text", "input_text": "second line" },
                        { "type": "input_file", "file_path": "  local.txt  " },
                        { "type": "input_file", "file_id": " file-123 " }
                    ]
                }
            ]
        }))
        .expect("request decodes");

        let flattened = flatten_messages(&request).expect("flattened messages");

        assert_eq!(flattened.prompt, "first line\nsecond line");
        for label in ["USER:", "SYSTEM:", "ASSISTANT:", "TOOL:", "DEVELOPER:"] {
            assert!(
                !flattened.prompt.contains(label),
                "prompt contained {label}"
            );
        }
        assert_eq!(flattened.file_paths, vec!["seed.txt", "local.txt"]);
        assert_eq!(flattened.file_ids, vec!["file-seed", "file-123"]);
    }

    #[test]
    fn single_user_message_prompt_is_unchanged() {
        let request: ChatCompletionsRequest = serde_json::from_value(serde_json::json!({
            "model": "tablinq-instant",
            "messages": [
                {
                    "role": "user",
                    "content": "raw user query unchanged"
                }
            ]
        }))
        .expect("request decodes");

        let flattened = flatten_messages(&request).expect("flattened messages");

        assert_eq!(flattened.prompt, "raw user query unchanged");
    }

    #[test]
    fn non_user_roles_are_rejected_before_flattening() {
        let request: ChatCompletionsRequest = serde_json::from_value(serde_json::json!({
            "model": "tablinq-instant",
            "messages": [
                {
                    "role": "system",
                    "content": "follow instructions"
                }
            ]
        }))
        .expect("request decodes");

        let err = flatten_messages(&request).expect_err("non-user role should fail");
        assert!(err.to_string().contains("only user messages"));
    }
}
