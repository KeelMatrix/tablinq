mod ansi;
mod execution;
mod markdown;
mod response;

pub use ansi::format_cli_error;
pub(crate) use execution::{
    execute_ask_with_terminal_presentation, execute_research_with_terminal_presentation,
};

#[cfg(test)]
mod tests;
