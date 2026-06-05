use super::CoreService;
use crate::browser::{ExecutionErrorMetadata, execution_error};
use crate::db::{NativeUploadBudgetRequest, NativeUploadBudgetReservation};
use anyhow::Result;
use chrono::Utc;
use shared_protocol::{
    AskCommand, ErrorCode, NativeUploadRateLimitDiagnostic, ResearchCommand,
    USER_PROMPT_INLINE_FALLBACK_THRESHOLD_CHARS, USER_PROMPT_UPLOAD_THRESHOLD_CHARS,
};

const MAX_COMPOSER_UPLOAD_FILES: usize = 10;
const NATIVE_UPLOAD_SCOPE_KEY: &str = "chatgpt-native-browser-session";

impl CoreService {
    pub(super) fn apply_native_upload_budget_to_ask(&self, command: &mut AskCommand) -> Result<()> {
        command.prompt_upload_threshold_chars = None;
        let prompt_chars = command.prompt.chars().count();
        let prompt_upload_cost = usize::from(
            prompt_chars > USER_PROMPT_UPLOAD_THRESHOLD_CHARS
                && command.files.len() < MAX_COMPOSER_UPLOAD_FILES,
        );
        let attempted_upload_cost = (command.files.len() + prompt_upload_cost) as u32;
        if attempted_upload_cost == 0 {
            return Ok(());
        }

        match self.reserve_native_upload_budget(attempted_upload_cost)? {
            NativeUploadBudgetReservation::Charged | NativeUploadBudgetReservation::NoUpload => {
                Ok(())
            }
            NativeUploadBudgetReservation::Exhausted(_diagnostic)
                if should_inline_prompt_upload_fallback(
                    command,
                    prompt_chars,
                    prompt_upload_cost,
                ) =>
            {
                command.prompt_upload_threshold_chars =
                    Some(USER_PROMPT_INLINE_FALLBACK_THRESHOLD_CHARS as u32);
                Ok(())
            }
            NativeUploadBudgetReservation::Exhausted(diagnostic) => {
                Err(native_upload_rate_limit_error(diagnostic))
            }
        }
    }

    pub(super) fn reserve_native_upload_budget_for_research(
        &self,
        command: &ResearchCommand,
    ) -> Result<()> {
        let attempted_upload_cost = command.files.len() as u32;
        if attempted_upload_cost == 0 {
            return Ok(());
        }

        match self.reserve_native_upload_budget(attempted_upload_cost)? {
            NativeUploadBudgetReservation::Charged | NativeUploadBudgetReservation::NoUpload => {
                Ok(())
            }
            NativeUploadBudgetReservation::Exhausted(diagnostic) => {
                Err(native_upload_rate_limit_error(diagnostic))
            }
        }
    }

    fn reserve_native_upload_budget(
        &self,
        attempted_upload_cost: u32,
    ) -> Result<NativeUploadBudgetReservation> {
        let mut db = self.open_db()?;
        db.reserve_native_upload_budget(NativeUploadBudgetRequest {
            scope_key: NATIVE_UPLOAD_SCOPE_KEY,
            configured_limit: self.config.native_upload_limit_count(),
            window_minutes: self.config.native_upload_limit_window_minutes(),
            attempted_upload_cost,
            now: Utc::now(),
        })
    }

    #[cfg(test)]
    pub(crate) fn current_native_upload_charged_uploads(&self) -> Result<u32> {
        let db = self.open_db()?;
        db.current_native_upload_charged_uploads(
            NATIVE_UPLOAD_SCOPE_KEY,
            self.config.native_upload_limit_window_minutes(),
            Utc::now(),
        )
    }
}

fn should_inline_prompt_upload_fallback(
    command: &AskCommand,
    prompt_chars: usize,
    prompt_upload_cost: usize,
) -> bool {
    command.files.is_empty()
        && prompt_upload_cost == 1
        && prompt_chars <= USER_PROMPT_INLINE_FALLBACK_THRESHOLD_CHARS
}

fn native_upload_rate_limit_error(diagnostic: NativeUploadRateLimitDiagnostic) -> anyhow::Error {
    let message = native_upload_rate_limit_message(&diagnostic);
    execution_error(
        ExecutionErrorMetadata::new(ErrorCode::NativeUploadRateLimited, message, None)
            .with_native_upload_rate_limit(diagnostic),
    )
}

pub(crate) fn native_upload_rate_limit_message(
    diagnostic: &NativeUploadRateLimitDiagnostic,
) -> String {
    format!(
        "Tablinq's native ChatGPT upload budget is exhausted: configured limit {} uploads per {} minutes; uploads currently charged {}; attempted upload cost {}; remaining time until release {}; reset timestamp {}.",
        diagnostic.configured_limit,
        diagnostic.window_minutes,
        diagnostic.uploads_currently_charged,
        diagnostic.attempted_upload_cost,
        format_duration(diagnostic.remaining_seconds_until_release),
        diagnostic.reset_at.to_rfc3339(),
    )
}

fn format_duration(total_seconds: u64) -> String {
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppConfig;
    use shared_protocol::{Mode, ModelClass};
    use tempfile::tempdir;

    fn test_core(limit: u32) -> (tempfile::TempDir, CoreService) {
        let dir = tempdir().unwrap();
        let mut config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        config.file.native_upload_limit_count = Some(limit);
        config.file.native_upload_limit_window_minutes = Some(180);
        let core = CoreService::new(config).unwrap();
        (dir, core)
    }

    fn ask_command(prompt: String, files: Vec<String>) -> AskCommand {
        AskCommand {
            prompt,
            conversation_id: None,
            new_conversation: false,
            mode: Mode::Normal,
            model_class: Some(ModelClass::Instant),
            effort: None,
            files,
            prompt_upload_threshold_chars: None,
            auto_recover_provider_generation_failure: true,
        }
    }

    #[test]
    fn explicit_file_uploads_charge_budget_before_execution() {
        let (_dir, core) = test_core(3);
        let prepared = core
            .prepare_ask(ask_command(
                "use files".to_owned(),
                vec!["one.txt".to_owned(), "two.txt".to_owned()],
            ))
            .unwrap();

        assert_eq!(prepared.command.files.len(), 2);
        assert_eq!(core.current_native_upload_charged_uploads().unwrap(), 2);

        let err = core
            .prepare_ask(ask_command(
                "next file".to_owned(),
                vec!["three.txt".to_owned()],
            ))
            .unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("uploads currently charged 2"));
        assert!(message.contains("attempted upload cost 1"));
    }

    #[test]
    fn long_prompt_text_upload_charges_one_upload() {
        let (_dir, core) = test_core(3);
        core.prepare_ask(ask_command(
            "x".repeat(USER_PROMPT_UPLOAD_THRESHOLD_CHARS + 1),
            vec![],
        ))
        .unwrap();

        assert_eq!(core.current_native_upload_charged_uploads().unwrap(), 1);
    }

    #[test]
    fn boundary_blocks_when_request_would_meet_configured_budget() {
        let (_dir, core) = test_core(2);
        core.prepare_ask(ask_command("first".to_owned(), vec!["one.txt".to_owned()]))
            .unwrap();

        let err = core
            .prepare_ask(ask_command("second".to_owned(), vec!["two.txt".to_owned()]))
            .unwrap_err();

        let message = format!("{err:#}");
        assert!(message.contains("configured limit 2 uploads per 180 minutes"));
        assert!(message.contains("uploads currently charged 1"));
        assert!(message.contains("attempted upload cost 1"));
        assert!(message.contains("reset timestamp "));
    }

    #[test]
    fn exhausted_budget_inlines_only_prompt_txt_fallback_under_24000_chars() {
        let (_dir, core) = test_core(2);
        core.prepare_ask(ask_command("first".to_owned(), vec!["one.txt".to_owned()]))
            .unwrap();

        let prepared = core
            .prepare_ask(ask_command(
                "x".repeat(USER_PROMPT_UPLOAD_THRESHOLD_CHARS + 1),
                vec![],
            ))
            .unwrap();

        assert_eq!(
            prepared.command.prompt_upload_threshold_chars,
            Some(USER_PROMPT_INLINE_FALLBACK_THRESHOLD_CHARS as u32)
        );
        assert_eq!(core.current_native_upload_charged_uploads().unwrap(), 1);
    }

    #[test]
    fn exhausted_budget_blocks_prompt_fallback_over_24000_chars() {
        let (_dir, core) = test_core(2);
        core.prepare_ask(ask_command("first".to_owned(), vec!["one.txt".to_owned()]))
            .unwrap();

        let err = core
            .prepare_ask(ask_command(
                "x".repeat(USER_PROMPT_INLINE_FALLBACK_THRESHOLD_CHARS + 1),
                vec![],
            ))
            .unwrap_err();

        assert!(format!("{err:#}").contains("native ChatGPT upload budget is exhausted"));
    }

    #[test]
    fn exhausted_budget_blocks_long_prompt_with_explicit_files() {
        let (_dir, core) = test_core(3);
        core.prepare_ask(ask_command(
            "first".to_owned(),
            vec!["one.txt".to_owned(), "two.txt".to_owned()],
        ))
        .unwrap();

        let err = core
            .prepare_ask(ask_command(
                "x".repeat(USER_PROMPT_UPLOAD_THRESHOLD_CHARS + 1),
                vec!["three.txt".to_owned()],
            ))
            .unwrap_err();

        let message = format!("{err:#}");
        assert!(message.contains("attempted upload cost 2"));
    }

    #[test]
    fn diagnostic_message_includes_rate_limit_reset_fields() {
        let diagnostic = NativeUploadRateLimitDiagnostic {
            configured_limit: 65,
            window_minutes: 180,
            uploads_currently_charged: 64,
            attempted_upload_cost: 1,
            remaining_seconds_until_release: 3_661,
            reset_at: chrono::DateTime::parse_from_rfc3339("2026-06-04T12:34:56Z")
                .unwrap()
                .with_timezone(&Utc),
        };

        let message = native_upload_rate_limit_message(&diagnostic);

        assert!(message.contains("configured limit 65 uploads per 180 minutes"));
        assert!(message.contains("uploads currently charged 64"));
        assert!(message.contains("attempted upload cost 1"));
        assert!(message.contains("remaining time until release 01:01:01"));
        assert!(message.contains("reset timestamp 2026-06-04T12:34:56+00:00"));
    }

    #[test]
    fn research_explicit_files_share_native_upload_budget() {
        let (_dir, core) = test_core(3);
        core.prepare_research(ResearchCommand {
            query: "research".to_owned(),
            conversation_id: None,
            new_conversation: false,
            files: vec!["one.txt".to_owned(), "two.txt".to_owned()],
            output_dir: None,
        })
        .unwrap();

        let err = core
            .prepare_ask(ask_command("ask".to_owned(), vec!["three.txt".to_owned()]))
            .unwrap_err();
        assert!(format!("{err:#}").contains("uploads currently charged 2"));
    }
}
