pub const PROTOCOL_VERSION: &str = "tablinq.v1";
pub const SELECTOR_PACK_FORMAT_VERSION: u32 = 1;
pub const CONFIG_VERSION: u32 = 2;
pub const APP_NAME: &str = "Tablinq";
pub const CHROME_ATTACH_PROFILE_DIR_NAME: &str = "chrome-attach-profile";
pub const CHROME_ATTACH_PROFILE_NOTE: &str = "Chrome attach-first uses a dedicated persistent real profile under the Tablinq state root. Sign in once in that profile before attaching.";

mod artifacts;
mod browser;
mod clear;
mod commands;
mod config;
mod conversations;
mod daemon;
mod events;
mod jobs;
mod native_uploads;
mod rate_limits;
mod recovery;
mod requests;
mod state_paths;
mod upload_limits;
mod validation;

pub use artifacts::*;
pub use browser::*;
pub use clear::*;
pub use commands::*;
pub use config::*;
pub use conversations::*;
pub use daemon::*;
pub use events::*;
pub use jobs::*;
pub use native_uploads::*;
pub use rate_limits::*;
pub use recovery::*;
pub use requests::*;
pub use state_paths::*;
pub use upload_limits::*;
pub use validation::ValidationError;

#[cfg(test)]
mod tests;
