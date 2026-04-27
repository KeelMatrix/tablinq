use anyhow::{Result, bail};

use super::request::{ChatCompletionsRequest, MessageContent};

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
        let mut text_parts = Vec::new();
        match &message.content {
            MessageContent::Text(text) => {
                if !text.trim().is_empty() {
                    text_parts.push(text.trim().to_owned());
                }
            }
            MessageContent::Parts(parts) => {
                for part in parts {
                    match part.kind.as_str() {
                        "text" => {
                            if let Some(text) = part.text.as_ref()
                                && !text.trim().is_empty()
                            {
                                text_parts.push(text.trim().to_owned());
                            }
                        }
                        "input_text" => {
                            if let Some(text) = part.input_text.as_ref()
                                && !text.trim().is_empty()
                            {
                                text_parts.push(text.trim().to_owned());
                            }
                        }
                        "input_file" => {
                            if let Some(file_id) = part.file_id.as_ref() {
                                if file_id.trim().is_empty() {
                                    bail!("input_file file_id must not be empty");
                                }
                                file_ids.push(file_id.trim().to_owned());
                            } else if let Some(file_path) = part.file_path.as_ref() {
                                if file_path.trim().is_empty() {
                                    bail!("input_file file_path must not be empty");
                                }
                                file_paths.push(file_path.trim().to_owned());
                            } else {
                                bail!("input_file parts must include file_id or file_path");
                            }
                        }
                        other => bail!("message content part type {other} is not supported"),
                    }
                }
            }
        }

        if !text_parts.is_empty() {
            sections.push(format!(
                "{}:\n{}",
                message.role.to_uppercase(),
                text_parts.join("\n")
            ));
        }
    }

    if sections.is_empty() {
        bail!("messages do not contain any supported text content");
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
    fn flattens_text_input_text_and_input_file_parts() {
        let request: ChatCompletionsRequest = serde_json::from_value(serde_json::json!({
            "model": "tablinq-instant",
            "tablinq_files": ["seed.txt"],
            "tablinq_file_ids": ["file-seed"],
            "messages": [
                {
                    "role": "system",
                    "content": "  follow instructions  "
                },
                {
                    "role": "user",
                    "content": [
                        { "type": "text", "text": "  first line  " },
                        { "type": "input_text", "input_text": "second line" },
                        { "type": "input_file", "file_path": "  local.txt  " },
                        { "type": "input_file", "file_id": " file-123 " }
                    ]
                }
            ]
        }))
        .expect("request decodes");

        let flattened = flatten_messages(&request).expect("flattened messages");

        assert_eq!(
            flattened.prompt,
            "SYSTEM:\nfollow instructions\n\nUSER:\nfirst line\nsecond line"
        );
        assert_eq!(flattened.file_paths, vec!["seed.txt", "local.txt"]);
        assert_eq!(flattened.file_ids, vec!["file-seed", "file-123"]);
    }
}
