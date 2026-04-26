use crate::http::RequestContext;
use crate::openai::{self, ChatCompletionsRequest, RequestRole, resolve_model};
use crate::uploads::UploadStore;
use anyhow::{Context, Result};
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
    let request: ChatCompletionsRequest =
        serde_json::from_slice(&context.request.body).context("decode chat completions request")?;
    openai::validate_chat_request(&request)?;
    let model = resolve_model(&request.model).context("resolve provider model")?;
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

    let job = if model.uses_research {
        PreparedChatJob::Research(core.prepare_research(shared_protocol::ResearchCommand {
            query: flattened.prompt,
            conversation_id,
            new_conversation,
            files,
            output_dir: None,
        })?)
    } else {
        PreparedChatJob::Ask(core.prepare_ask(shared_protocol::AskCommand {
            prompt: flattened.prompt,
            conversation_id,
            new_conversation,
            mode: model.mode,
            model_class: model.model_class,
            effort: model.effort,
            files,
            auto_recover_provider_generation_failure:
                request.tablinq_auto_recover_provider_generation_failure,
        })?)
    };

    Ok(PreparedChatRequest {
        request,
        model_id: model.id,
        request_role,
        parent_request_id,
        file_ids: flattened.file_ids,
        job,
    })
}
