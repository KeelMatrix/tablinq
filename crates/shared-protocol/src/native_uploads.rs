use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const USER_PROMPT_UPLOAD_THRESHOLD_CHARS: usize = 20_000;
pub const USER_PROMPT_INLINE_FALLBACK_THRESHOLD_CHARS: usize = 24_000;
pub const NATIVE_UPLOAD_LIMIT_DEFAULT_COUNT: u32 = 65;
pub const NATIVE_UPLOAD_LIMIT_DEFAULT_WINDOW_MINUTES: u32 = 180;
pub const NATIVE_UPLOAD_LIMIT_HARD_MAX_COUNT: u32 = 80;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NativeUploadRateLimitDiagnostic {
    pub configured_limit: u32,
    pub window_minutes: u32,
    pub uploads_currently_charged: u32,
    pub attempted_upload_cost: u32,
    pub remaining_seconds_until_release: u64,
    pub reset_at: DateTime<Utc>,
}
