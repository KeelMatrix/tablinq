use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const CHATGPT_INSTANT_MESSAGE_LIMIT_DEFAULT_COUNT: u32 = 150;
pub const CHATGPT_INSTANT_MESSAGE_LIMIT_DEFAULT_WINDOW_MINUTES: u32 = 180;
pub const CHATGPT_THINKING_MESSAGE_LIMIT_DEFAULT_COUNT: u32 = 2_900;
pub const CHATGPT_THINKING_MESSAGE_LIMIT_DEFAULT_WINDOW_MINUTES: u32 = 10_080;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitBucket {
    Instant,
    Thinking,
    NativeUpload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResetRateLimitCommand {
    pub bucket: RateLimitBucket,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResetRateLimitResult {
    pub bucket: RateLimitBucket,
    pub deleted_attempts: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChatGptMessageRateLimitDiagnostic {
    pub bucket: RateLimitBucket,
    pub configured_limit: u32,
    pub window_minutes: u32,
    pub messages_currently_charged: u32,
    pub attempted_message_cost: u32,
    pub remaining_seconds_until_release: u64,
    pub reset_at: DateTime<Utc>,
}
