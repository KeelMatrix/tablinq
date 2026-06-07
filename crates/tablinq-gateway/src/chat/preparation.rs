use crate::http::RequestContext;
use crate::openai::{
    self, ChatCompletionsRequest, ProviderRequestTarget, RESEARCH_PRESET_ID, RequestRole,
    invalid_request_error, invalid_request_json_error, resolve_request_target,
};
use crate::uploads::UploadStore;
use anyhow::Result;
use gptd::CoreService;
use gptd::core::{PreparedAskJob, PreparedResearchJob};

pub(crate) enum PreparedChatJob {
    Ask(PreparedAskJob),
    Research(PreparedResearchJob),
}

pub(crate) struct PreparedChatRequest {
    pub(crate) request: ChatCompletionsRequest,
    pub(crate) model_id: &'static str,
    pub(crate) request_role: RequestRole,
    pub(crate) parent_request_id: Option<String>,
    pub(crate) file_ids: Vec<String>,
    pub(crate) job: PreparedChatJob,
}

impl PreparedChatRequest {
    pub(crate) fn job_id(&self) -> &str {
        match &self.job {
            PreparedChatJob::Ask(job) => &job.job_id,
            PreparedChatJob::Research(job) => &job.job_id,
        }
    }

    pub(crate) fn mode(&self) -> shared_protocol::Mode {
        match &self.job {
            PreparedChatJob::Ask(job) => job.command.mode,
            PreparedChatJob::Research(_) => shared_protocol::Mode::Research,
        }
    }

    pub(crate) fn conversation_id_hint(&self) -> Option<&str> {
        match &self.job {
            PreparedChatJob::Ask(job) => job.command.conversation_id.as_deref(),
            PreparedChatJob::Research(job) => job.command.conversation_id.as_deref(),
        }
    }
}

pub(crate) fn prepare_chat_completions_request(
    core: &CoreService,
    upload_store: &UploadStore,
    context: &RequestContext,
) -> Result<PreparedChatRequest> {
    require_json_content_type(context)?;
    let request: ChatCompletionsRequest = serde_json::from_slice(&context.request.body)
        .map_err(|err| invalid_request_json_error(format!("invalid JSON body: {err}")))?;
    openai::validate_chat_request(&request)?;
    let target = resolve_request_target(&request.model).ok_or_else(|| {
        invalid_request_error(
            "unsupported_model",
            format!("model {} is not supported", request.model),
        )
    })?;
    let request_mode = request.resolved_tablinq_mode()?;
    let request_role = request.request_role();
    let parent_request_id = request.tablinq_parent_request_id.clone();
    let flattened = openai::flatten_messages(&request)?;
    let files = upload_store.resolve_file_inputs(flattened.file_paths, &flattened.file_ids)?;
    let conversation_id = request
        .tablinq_conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let new_conversation = request.tablinq_new_conversation;

    let (model_id, job) = match target {
        ProviderRequestTarget::Research => {
            if request_mode == shared_protocol::Mode::Search {
                return Err(invalid_request_error(
                    "unsupported_tablinq_mode",
                    "tablinq_mode=\"search\" applies to ask requests. Use model=<base target> with tablinq_mode=\"search\", or use tablinq-research without search mode for the dedicated research flow.",
                )
                .into());
            }
            (
                RESEARCH_PRESET_ID,
                PreparedChatJob::Research(core.prepare_research(
                    shared_protocol::ResearchCommand {
                        query: flattened.prompt,
                        conversation_id,
                        new_conversation,
                        files,
                        output_dir: None,
                    },
                )?),
            )
        }
        ProviderRequestTarget::Ask(model) => (
            model.id,
            PreparedChatJob::Ask(core.prepare_ask(shared_protocol::AskCommand {
                prompt: flattened.prompt,
                conversation_id,
                new_conversation,
                mode: request_mode,
                model_class: Some(model.model_class),
                effort: model.effort,
                files,
                prompt_upload_threshold_chars: None,
                prompt_upload_file_name: request.tablinq_prompt_upload_file_name.clone(),
                prompt_upload_composer_text: request.tablinq_prompt_upload_composer_text.clone(),
                auto_recover_provider_generation_failure:
                    request.tablinq_auto_recover_provider_generation_failure,
            })?),
        ),
    };

    Ok(PreparedChatRequest {
        request,
        model_id,
        request_role,
        parent_request_id,
        file_ids: flattened.file_ids,
        job,
    })
}

fn require_json_content_type(context: &RequestContext) -> Result<()> {
    let content_type = context.request.headers.get("content-type").map(|value| {
        value
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
    });
    if content_type.as_deref() == Some("application/json") {
        return Ok(());
    }

    Err(invalid_request_error(
        "unsupported_content_type",
        "POST /v1/chat/completions requires content-type application/json",
    )
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::map_openai_error;
    use crate::http::HttpRequest;
    use gptd::AppConfig;
    use serde_json::{Value, json};
    use shared_protocol::{AskCommand, Effort, Mode, ModelClass};
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn prepare_ask_command_from_body(body: Value) -> Result<AskCommand> {
        let dir = tempdir()?;
        let config = AppConfig::load_or_create(Some(dir.path()))?;
        let core = CoreService::new(config.clone())?;
        let upload_store = UploadStore::load(&config)?;
        let context = RequestContext {
            request_id: "req-prepare".to_owned(),
            request: HttpRequest {
                method: "POST".to_owned(),
                path: "/v1/chat/completions".to_owned(),
                headers: BTreeMap::from([(
                    "content-type".to_owned(),
                    "application/json".to_owned(),
                )]),
                body: serde_json::to_vec(&body)?,
            },
        };

        let prepared = prepare_chat_completions_request(&core, &upload_store, &context)?;
        match prepared.job {
            PreparedChatJob::Ask(job) => Ok(job.command),
            PreparedChatJob::Research(_) => unreachable!("expected ask job"),
        }
    }

    fn prepare_ask_command(model: &str, tablinq_mode: Option<&str>) -> Result<AskCommand> {
        let mut body = json!({
            "model": model,
            "messages": [{ "role": "user", "content": "hello" }]
        });
        if let Some(tablinq_mode) = tablinq_mode {
            body["tablinq_mode"] = json!(tablinq_mode);
        }
        prepare_ask_command_from_body(body)
    }

    fn assert_ask_selection(
        model: &str,
        tablinq_mode: Option<&str>,
        expected_mode: Mode,
        expected_model_class: ModelClass,
        expected_effort: Option<Effort>,
    ) {
        let command = prepare_ask_command(model, tablinq_mode).expect("request prepares");
        assert_eq!(command.mode, expected_mode);
        assert_eq!(command.model_class, Some(expected_model_class));
        assert_eq!(command.effort, expected_effort);
    }

    #[test]
    fn search_mode_keeps_thinking_standard_base_model_selection() {
        assert_ask_selection(
            "tablinq-thinking-standard",
            Some("search"),
            Mode::Search,
            ModelClass::Thinking,
            Some(Effort::Standard),
        );
    }

    #[test]
    fn search_mode_keeps_thinking_extended_base_model_selection() {
        assert_ask_selection(
            "tablinq-thinking-extended",
            Some("search"),
            Mode::Search,
            ModelClass::Thinking,
            Some(Effort::Extended),
        );
    }

    #[test]
    fn search_mode_combines_with_instant_base_model_selection() {
        assert_ask_selection(
            "tablinq-instant",
            Some("search"),
            Mode::Search,
            ModelClass::Instant,
            None,
        );
    }

    #[test]
    fn omitted_tablinq_mode_defaults_to_normal_mode() {
        assert_ask_selection(
            "tablinq-thinking-standard",
            None,
            Mode::Normal,
            ModelClass::Thinking,
            Some(Effort::Standard),
        );
    }

    #[test]
    fn legacy_request_leaves_prompt_upload_fields_absent() {
        let command = prepare_ask_command("tablinq-instant", None).expect("request prepares");

        assert_eq!(command.prompt, "hello");
        assert_eq!(command.prompt_upload_file_name, None);
        assert_eq!(command.prompt_upload_composer_text, None);
    }

    #[test]
    fn explicit_prompt_upload_fields_map_through_prepared_ask_with_files_and_parts() {
        let command = prepare_ask_command_from_body(json!({
            "model": "tablinq-instant",
            "tablinq_files": ["top-level.txt"],
            "tablinq_prompt_upload_file_name": "openclaude-large-prompt.txt",
            "tablinq_prompt_upload_composer_text": "Read openclaude-large-prompt.txt exactly.",
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": "first" },
                    { "type": "input_text", "input_text": "second" },
                    { "type": "input_file", "file_path": "part-path.txt" }
                ]
            }]
        }))
        .expect("request prepares");

        assert_eq!(command.prompt, "first\nsecond");
        assert_eq!(
            command.prompt_upload_file_name.as_deref(),
            Some("openclaude-large-prompt.txt")
        );
        assert_eq!(
            command.prompt_upload_composer_text.as_deref(),
            Some("Read openclaude-large-prompt.txt exactly.")
        );
        assert_eq!(command.files, vec!["top-level.txt", "part-path.txt"]);
    }

    #[test]
    fn explicit_prompt_upload_fields_preserve_staged_file_ids() -> Result<()> {
        let dir = tempdir()?;
        let config = AppConfig::load_or_create(Some(dir.path()))?;
        let core = CoreService::new(config.clone())?;
        let upload_store = UploadStore::load(&config)?;
        let staged = upload_store.stage_upload("staged.txt", Some("text/plain"), b"staged")?;
        let staged_file_id = staged.file_id.clone();
        let body = json!({
            "model": "tablinq-instant",
            "tablinq_file_ids": [staged_file_id],
            "tablinq_prompt_upload_file_name": "openclaude-large-prompt.txt",
            "tablinq_prompt_upload_composer_text": "Read openclaude-large-prompt.txt exactly.",
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "text", "text": "first" },
                    { "type": "input_file", "file_id": staged.file_id }
                ]
            }]
        });
        let context = RequestContext {
            request_id: "req-prepare-file-id".to_owned(),
            request: HttpRequest {
                method: "POST".to_owned(),
                path: "/v1/chat/completions".to_owned(),
                headers: BTreeMap::from([(
                    "content-type".to_owned(),
                    "application/json".to_owned(),
                )]),
                body: serde_json::to_vec(&body)?,
            },
        };

        let prepared = prepare_chat_completions_request(&core, &upload_store, &context)?;
        let PreparedChatJob::Ask(job) = prepared.job else {
            unreachable!("expected ask job");
        };

        assert_eq!(job.command.files.len(), 2);
        assert!(
            job.command
                .files
                .iter()
                .all(|path| path.ends_with("staged.txt"))
        );
        assert_eq!(
            job.command.prompt_upload_file_name.as_deref(),
            Some("openclaude-large-prompt.txt")
        );
        assert_eq!(
            job.command.prompt_upload_composer_text.as_deref(),
            Some("Read openclaude-large-prompt.txt exactly.")
        );
        assert_eq!(prepared.file_ids.len(), 2);
        Ok(())
    }

    #[test]
    fn explicit_normal_tablinq_mode_prepares_normal_mode() {
        assert_ask_selection(
            "tablinq-thinking-standard",
            Some("normal"),
            Mode::Normal,
            ModelClass::Thinking,
            Some(Effort::Standard),
        );
    }

    #[test]
    fn invalid_tablinq_mode_is_rejected_before_queueing() {
        let err = prepare_ask_command("tablinq-thinking-standard", Some("browse"))
            .expect_err("invalid tablinq_mode should fail");

        assert!(err.to_string().contains("tablinq_mode"));
        assert!(err.to_string().contains("normal"));
        assert!(err.to_string().contains("search"));
    }

    #[test]
    fn legacy_search_model_id_is_rejected_before_queueing() {
        let err = prepare_ask_command("tablinq-search", None)
            .expect_err("legacy search model should fail");

        assert!(err.to_string().contains("tablinq-search is not a model"));
        assert!(err.to_string().contains("tablinq_mode=\"search\""));
    }

    #[test]
    fn direct_cli_and_gateway_ask_preparation_share_native_upload_budget() -> Result<()> {
        let dir = tempdir()?;
        let mut config = AppConfig::load_or_create(Some(dir.path()))?;
        config.file.native_upload_limit_count = Some(2);
        config.file.native_upload_limit_window_minutes = Some(180);
        let core = CoreService::new(config.clone())?;
        let upload_store = UploadStore::load(&config)?;

        core.prepare_ask(AskCommand {
            prompt: "cli path".to_owned(),
            conversation_id: None,
            new_conversation: false,
            mode: Mode::Normal,
            model_class: Some(ModelClass::Instant),
            effort: None,
            files: vec!["cli.txt".to_owned()],
            prompt_upload_threshold_chars: None,
            prompt_upload_file_name: None,
            prompt_upload_composer_text: None,
            auto_recover_provider_generation_failure: true,
        })?;

        let body = json!({
            "model": "tablinq-instant",
            "tablinq_files": ["api.txt"],
            "messages": [{ "role": "user", "content": "api path" }]
        });
        let context = RequestContext {
            request_id: "req-upload-budget".to_owned(),
            request: HttpRequest {
                method: "POST".to_owned(),
                path: "/v1/chat/completions".to_owned(),
                headers: BTreeMap::from([(
                    "content-type".to_owned(),
                    "application/json".to_owned(),
                )]),
                body: serde_json::to_vec(&body)?,
            },
        };

        let err = match prepare_chat_completions_request(&core, &upload_store, &context) {
            Ok(_) => panic!("gateway ask should see CLI upload charge"),
            Err(err) => err,
        };
        let response = map_openai_error(&context.request_id, &err, None);
        let body: Value = serde_json::from_slice(&response.body)?;

        assert_eq!(response.status_code, 429);
        assert_eq!(body["error"]["type"], "rate_limit_error");
        assert_eq!(body["error"]["code"], "native_upload_budget_exhausted");
        assert_eq!(body["tablinq_native_upload_budget"]["configured_limit"], 2);
        assert_eq!(
            body["tablinq_native_upload_budget"]["uploads_currently_charged"],
            1
        );
        assert_eq!(
            body["tablinq_native_upload_budget"]["attempted_upload_cost"],
            1
        );
        assert!(
            body["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("reset timestamp"))
        );
        Ok(())
    }
}
