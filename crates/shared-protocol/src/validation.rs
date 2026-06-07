use crate::{
    MAX_ATTACHMENT_FILES, MAX_SAFE_FILE_NAME_BYTES, PROTOCOL_VERSION, max_upload_file_bytes,
};
use std::fs;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("{0} must not be empty")]
    EmptyField(&'static str),
    #[error("{0} must contain at least one item")]
    EmptyCollection(&'static str),
    #[error("{field} must contain at most {max} items; found {found}")]
    TooManyItems {
        field: &'static str,
        max: usize,
        found: usize,
    },
    #[error("{field} file exceeds {max_bytes} bytes: {path} ({found_bytes} bytes)")]
    FileTooLarge {
        field: &'static str,
        path: String,
        max_bytes: u64,
        found_bytes: u64,
    },
    #[error("{field} is invalid: {message}")]
    InvalidFieldCombination { field: String, message: String },
    #[error("selector pack format version {found} is unsupported; expected {expected}")]
    UnsupportedSelectorPackFormat { found: u32, expected: u32 },
    #[error("{field} contains duplicate strategy id {strategy_id}")]
    DuplicateStrategy { field: String, strategy_id: String },
    #[error("protocol version {found} is unsupported; expected {expected}")]
    UnsupportedVersion {
        found: String,
        expected: &'static str,
    },
}

pub(crate) fn validate_exact_version(version: &str) -> Result<(), ValidationError> {
    if version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(ValidationError::UnsupportedVersion {
            found: version.to_owned(),
            expected: PROTOCOL_VERSION,
        })
    }
}

pub(crate) fn validate_required(field: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() {
        Err(ValidationError::EmptyField(field))
    } else {
        Ok(())
    }
}

pub(crate) fn validate_optional(
    field: &'static str,
    value: Option<&str>,
) -> Result<(), ValidationError> {
    match value {
        Some(value) => validate_required(field, value),
        None => Ok(()),
    }
}

pub(crate) fn validate_paths(
    field: &'static str,
    values: &[String],
) -> Result<(), ValidationError> {
    if values.len() > MAX_ATTACHMENT_FILES {
        return Err(ValidationError::TooManyItems {
            field,
            max: MAX_ATTACHMENT_FILES,
            found: values.len(),
        });
    }
    let max_file_bytes = max_upload_file_bytes();
    for value in values {
        validate_required(field, value)?;
        if let Ok(metadata) = fs::metadata(value)
            && metadata.is_file()
            && metadata.len() > max_file_bytes
        {
            return Err(ValidationError::FileTooLarge {
                field,
                path: value.clone(),
                max_bytes: max_file_bytes,
                found_bytes: metadata.len(),
            });
        }
    }
    Ok(())
}

pub fn validate_safe_txt_basename(field: &'static str, value: &str) -> Result<(), ValidationError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ValidationError::EmptyField(field));
    }
    if trimmed != value {
        return Err(invalid_field(
            field,
            "must not include leading or trailing whitespace",
        ));
    }
    if value.len() > MAX_SAFE_FILE_NAME_BYTES {
        return Err(invalid_field(
            field,
            format!("must be at most {MAX_SAFE_FILE_NAME_BYTES} bytes"),
        ));
    }
    if !value.ends_with(".txt") {
        return Err(invalid_field(field, "must end with .txt"));
    }
    if value.contains('/') || value.contains('\\') {
        return Err(invalid_field(
            field,
            "must be a basename without path separators",
        ));
    }
    if value == "." || value == ".." || value.contains("..") {
        return Err(invalid_field(field, "must not contain traversal segments"));
    }
    if value.ends_with('.') || value.ends_with(' ') {
        return Err(invalid_field(field, "must not end with a dot or space"));
    }
    if value
        .chars()
        .any(|ch| matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*') || ch.is_control())
    {
        return Err(invalid_field(
            field,
            "must not contain characters unsafe for exact file upload names",
        ));
    }
    let stem = value.trim_end_matches(".txt");
    if is_windows_reserved_basename(stem) {
        return Err(invalid_field(field, "must not use a reserved device name"));
    }
    Ok(())
}

fn invalid_field(field: &'static str, message: impl Into<String>) -> ValidationError {
    ValidationError::InvalidFieldCombination {
        field: field.to_owned(),
        message: message.into(),
    }
}

fn is_windows_reserved_basename(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || upper
        .strip_prefix("COM")
        .and_then(|suffix| suffix.parse::<u8>().ok())
        .is_some_and(|value| (1..=9).contains(&value))
        || upper
            .strip_prefix("LPT")
            .and_then(|suffix| suffix.parse::<u8>().ok())
            .is_some_and(|value| (1..=9).contains(&value))
}
