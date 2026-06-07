use super::CoreService;
use crate::browser::{ExecutionErrorMetadata, execution_error};
use crate::db::{ChatGptMessageBudgetRequest, ChatGptMessageBudgetReservation};
use anyhow::Result;
use chrono::Utc;
use shared_protocol::{
    AskCommand, ChatGptMessageRateLimitDiagnostic, ErrorCode, ModelClass, RateLimitBucket,
    ResetRateLimitCommand, ResetRateLimitResult,
};

pub(in crate::core) const CHATGPT_INSTANT_MESSAGE_SCOPE_KEY: &str = "chatgpt-message-instant";
pub(in crate::core) const CHATGPT_THINKING_MESSAGE_SCOPE_KEY: &str = "chatgpt-message-thinking";
const ATTEMPTED_MESSAGE_COST: u32 = 1;

#[derive(Debug, Clone, Copy)]
pub(in crate::core) struct MessageBudgetScope {
    pub(in crate::core) scope_key: &'static str,
    pub(in crate::core) bucket: RateLimitBucket,
    pub(in crate::core) configured_limit: u32,
    pub(in crate::core) window_minutes: u32,
}

impl CoreService {
    pub(super) fn apply_chatgpt_message_budget_to_ask(&self, command: &AskCommand) -> Result<()> {
        let scope = self.chatgpt_message_budget_scope(command);
        let mut db = self.open_db()?;
        match db.reserve_chatgpt_message_budget(ChatGptMessageBudgetRequest {
            scope_key: scope.scope_key,
            bucket: scope.bucket,
            configured_limit: scope.configured_limit,
            window_minutes: scope.window_minutes,
            attempted_message_cost: ATTEMPTED_MESSAGE_COST,
            now: Utc::now(),
        })? {
            ChatGptMessageBudgetReservation::Charged => Ok(()),
            ChatGptMessageBudgetReservation::Exhausted(diagnostic) => {
                Err(chatgpt_message_rate_limit_error(diagnostic))
            }
        }
    }

    pub fn reset_rate_limit(&self, command: ResetRateLimitCommand) -> Result<ResetRateLimitResult> {
        let mut db = self.open_db()?;
        let deleted_attempts = match command.bucket {
            RateLimitBucket::Instant => {
                db.reset_chatgpt_message_budget(CHATGPT_INSTANT_MESSAGE_SCOPE_KEY)?
            }
            RateLimitBucket::Thinking => {
                db.reset_chatgpt_message_budget(CHATGPT_THINKING_MESSAGE_SCOPE_KEY)?
            }
            RateLimitBucket::NativeUpload => {
                db.reset_native_upload_budget(super::native_upload_limit::NATIVE_UPLOAD_SCOPE_KEY)?
            }
        };
        Ok(ResetRateLimitResult {
            bucket: command.bucket,
            deleted_attempts,
        })
    }

    #[cfg(test)]
    pub(crate) fn current_chatgpt_message_charged(&self, bucket: RateLimitBucket) -> Result<u32> {
        let scope = self.chatgpt_message_budget_scope_for_bucket(bucket);
        let db = self.open_db()?;
        db.current_chatgpt_messages_charged(scope.scope_key, scope.window_minutes, Utc::now())
    }

    fn chatgpt_message_budget_scope(&self, command: &AskCommand) -> MessageBudgetScope {
        let model_class = command.model_class.unwrap_or(ModelClass::Instant);
        match model_class {
            ModelClass::Instant => {
                self.chatgpt_message_budget_scope_for_bucket(RateLimitBucket::Instant)
            }
            ModelClass::Thinking => {
                self.chatgpt_message_budget_scope_for_bucket(RateLimitBucket::Thinking)
            }
        }
    }

    fn chatgpt_message_budget_scope_for_bucket(
        &self,
        bucket: RateLimitBucket,
    ) -> MessageBudgetScope {
        match bucket {
            RateLimitBucket::Instant => MessageBudgetScope {
                scope_key: CHATGPT_INSTANT_MESSAGE_SCOPE_KEY,
                bucket,
                configured_limit: self.config.chatgpt_instant_message_limit_count(),
                window_minutes: self.config.chatgpt_instant_message_limit_window_minutes(),
            },
            RateLimitBucket::Thinking => MessageBudgetScope {
                scope_key: CHATGPT_THINKING_MESSAGE_SCOPE_KEY,
                bucket,
                configured_limit: self.config.chatgpt_thinking_message_limit_count(),
                window_minutes: self.config.chatgpt_thinking_message_limit_window_minutes(),
            },
            RateLimitBucket::NativeUpload => unreachable!("native upload is not a message budget"),
        }
    }
}

fn chatgpt_message_rate_limit_error(
    diagnostic: ChatGptMessageRateLimitDiagnostic,
) -> anyhow::Error {
    let message = chatgpt_message_rate_limit_message(&diagnostic);
    execution_error(
        ExecutionErrorMetadata::new(ErrorCode::ChatGptMessageRateLimited, message, None)
            .with_chatgpt_message_rate_limit(diagnostic),
    )
}

pub(crate) fn chatgpt_message_rate_limit_message(
    diagnostic: &ChatGptMessageRateLimitDiagnostic,
) -> String {
    let bucket = match diagnostic.bucket {
        RateLimitBucket::Instant => "instant",
        RateLimitBucket::Thinking => "thinking",
        RateLimitBucket::NativeUpload => "native_upload",
    };
    format!(
        "Tablinq's ChatGPT {bucket} message budget is exhausted: configured limit {} messages per {} minutes; messages currently charged {}; attempted message cost {}; remaining time until release {}; reset timestamp {}.",
        diagnostic.configured_limit,
        diagnostic.window_minutes,
        diagnostic.messages_currently_charged,
        diagnostic.attempted_message_cost,
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
    use shared_protocol::{Effort, Mode};
    use tempfile::tempdir;

    fn test_core(instant_limit: u32, thinking_limit: u32) -> (tempfile::TempDir, CoreService) {
        let dir = tempdir().unwrap();
        let mut config = AppConfig::load_or_create(Some(dir.path())).unwrap();
        config.file.chatgpt_instant_message_limit_count = Some(instant_limit);
        config.file.chatgpt_instant_message_limit_window_minutes = Some(180);
        config.file.chatgpt_thinking_message_limit_count = Some(thinking_limit);
        config.file.chatgpt_thinking_message_limit_window_minutes = Some(10_080);
        let core = CoreService::new(config).unwrap();
        (dir, core)
    }

    fn ask_command(model_class: ModelClass, effort: Option<Effort>) -> AskCommand {
        AskCommand {
            prompt: "hello".to_owned(),
            conversation_id: None,
            new_conversation: false,
            mode: Mode::Normal,
            model_class: Some(model_class),
            effort,
            files: vec![],
            prompt_upload_threshold_chars: None,
            auto_recover_provider_generation_failure: true,
        }
    }

    #[test]
    fn instant_budget_allows_configured_count_and_blocks_next_attempt() {
        let (_dir, core) = test_core(2, 10);
        core.prepare_ask(ask_command(ModelClass::Instant, None))
            .unwrap();
        core.prepare_ask(ask_command(ModelClass::Instant, None))
            .unwrap();
        assert_eq!(
            core.current_chatgpt_message_charged(RateLimitBucket::Instant)
                .unwrap(),
            2
        );

        let err = core
            .prepare_ask(ask_command(ModelClass::Instant, None))
            .unwrap_err();
        let message = format!("{err:#}");
        assert!(message.contains("ChatGPT instant message budget is exhausted"));
        assert!(message.contains("configured limit 2 messages per 180 minutes"));
        assert!(message.contains("messages currently charged 2"));
    }

    #[test]
    fn thinking_standard_and_extended_share_budget() {
        let (_dir, core) = test_core(10, 2);
        core.prepare_ask(ask_command(ModelClass::Thinking, Some(Effort::Standard)))
            .unwrap();
        core.prepare_ask(ask_command(ModelClass::Thinking, Some(Effort::Extended)))
            .unwrap();
        assert_eq!(
            core.current_chatgpt_message_charged(RateLimitBucket::Thinking)
                .unwrap(),
            2
        );

        let err = core
            .prepare_ask(ask_command(ModelClass::Thinking, Some(Effort::Standard)))
            .unwrap_err();
        assert!(format!("{err:#}").contains("ChatGPT thinking message budget is exhausted"));
    }

    #[test]
    fn message_budget_persists_across_core_restarts() {
        let (dir, core) = test_core(1, 10);
        let config = core.config().clone();
        core.prepare_ask(ask_command(ModelClass::Instant, None))
            .unwrap();
        drop(core);

        let restarted = CoreService::new(config).unwrap();
        let err = restarted
            .prepare_ask(ask_command(ModelClass::Instant, None))
            .unwrap_err();
        assert!(format!("{err:#}").contains("messages currently charged 1"));
        drop(dir);
    }

    #[test]
    fn resetting_message_limit_clears_only_selected_bucket() {
        let (_dir, core) = test_core(10, 10);
        core.prepare_ask(ask_command(ModelClass::Instant, None))
            .unwrap();
        core.prepare_ask(ask_command(ModelClass::Thinking, Some(Effort::Standard)))
            .unwrap();

        let result = core
            .reset_rate_limit(ResetRateLimitCommand {
                bucket: RateLimitBucket::Instant,
            })
            .unwrap();
        assert_eq!(result.deleted_attempts, 1);
        assert_eq!(
            core.current_chatgpt_message_charged(RateLimitBucket::Instant)
                .unwrap(),
            0
        );
        assert_eq!(
            core.current_chatgpt_message_charged(RateLimitBucket::Thinking)
                .unwrap(),
            1
        );
    }

    #[test]
    fn missing_model_class_uses_instant_budget() {
        let (_dir, core) = test_core(10, 10);
        let mut command = ask_command(ModelClass::Thinking, Some(Effort::Standard));
        command.model_class = None;
        command.effort = Some(Effort::Extended);

        core.prepare_ask(command).unwrap();

        assert_eq!(
            core.current_chatgpt_message_charged(RateLimitBucket::Instant)
                .unwrap(),
            1
        );
        assert_eq!(
            core.current_chatgpt_message_charged(RateLimitBucket::Thinking)
                .unwrap(),
            0
        );
    }

    #[test]
    fn instant_and_thinking_budgets_are_independent() {
        let (_dir, core) = test_core(1, 1);

        core.prepare_ask(ask_command(ModelClass::Instant, None))
            .unwrap();
        core.prepare_ask(ask_command(ModelClass::Thinking, Some(Effort::Extended)))
            .unwrap();

        let instant_err = core
            .prepare_ask(ask_command(ModelClass::Instant, None))
            .unwrap_err();
        let thinking_err = core
            .prepare_ask(ask_command(ModelClass::Thinking, Some(Effort::Standard)))
            .unwrap_err();

        assert!(format!("{instant_err:#}").contains("ChatGPT instant message budget is exhausted"));
        assert!(
            format!("{thinking_err:#}").contains("ChatGPT thinking message budget is exhausted")
        );
    }

    #[test]
    fn resetting_thinking_limit_clears_only_thinking_bucket() {
        let (_dir, core) = test_core(10, 10);
        core.prepare_ask(ask_command(ModelClass::Instant, None))
            .unwrap();
        core.prepare_ask(ask_command(ModelClass::Thinking, Some(Effort::Extended)))
            .unwrap();

        let result = core
            .reset_rate_limit(ResetRateLimitCommand {
                bucket: RateLimitBucket::Thinking,
            })
            .unwrap();

        assert_eq!(result.deleted_attempts, 1);
        assert_eq!(
            core.current_chatgpt_message_charged(RateLimitBucket::Instant)
                .unwrap(),
            1
        );
        assert_eq!(
            core.current_chatgpt_message_charged(RateLimitBucket::Thinking)
                .unwrap(),
            0
        );
    }

    #[test]
    fn diagnostic_message_includes_reset_fields() {
        let diagnostic = ChatGptMessageRateLimitDiagnostic {
            bucket: RateLimitBucket::Thinking,
            configured_limit: 2_900,
            window_minutes: 10_080,
            messages_currently_charged: 2_900,
            attempted_message_cost: 1,
            remaining_seconds_until_release: 3_661,
            reset_at: chrono::DateTime::parse_from_rfc3339("2026-06-04T12:34:56Z")
                .unwrap()
                .with_timezone(&Utc),
        };

        let message = chatgpt_message_rate_limit_message(&diagnostic);

        assert!(message.contains("configured limit 2900 messages per 10080 minutes"));
        assert!(message.contains("messages currently charged 2900"));
        assert!(message.contains("attempted message cost 1"));
        assert!(message.contains("remaining time until release 01:01:01"));
        assert!(message.contains("reset timestamp 2026-06-04T12:34:56+00:00"));
    }
}
