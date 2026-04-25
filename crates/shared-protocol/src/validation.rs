use crate::PROTOCOL_VERSION;
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

pub(crate) const MAX_ATTACHMENT_FILES: usize = 10;
pub const MAX_ATTACHMENT_FILE_BYTES: u64 = 20 * 1024 * 1024;

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
    for value in values {
        validate_required(field, value)?;
        if let Ok(metadata) = fs::metadata(value)
            && metadata.is_file()
            && metadata.len() > MAX_ATTACHMENT_FILE_BYTES
        {
            return Err(ValidationError::FileTooLarge {
                field,
                path: value.clone(),
                max_bytes: MAX_ATTACHMENT_FILE_BYTES,
                found_bytes: metadata.len(),
            });
        }
    }
    Ok(())
}
