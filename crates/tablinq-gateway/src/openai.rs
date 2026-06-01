mod error_object;
mod message;
mod metadata;
mod models;
mod request;

pub use error_object::openai_error;
pub use message::flatten_messages;
pub use metadata::{provider_capability_catalog, request_role_name};
pub use models::{
    PROVIDER_MODELS, ProviderRequestTarget, RESEARCH_PRESET_ID, model_object,
    resolve_request_target,
};
pub use request::{
    ChatCompletionsRequest, OpenAiRequestError, RequestRole, invalid_request_error,
    invalid_request_json_error, validate_chat_request,
};
