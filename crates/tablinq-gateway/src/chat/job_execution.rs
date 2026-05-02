use super::preparation::{PreparedChatJob, PreparedChatRequest};
use anyhow::Result;
use gptd::CoreService;
use shared_protocol::{EventPayload, RecoveryPacket, RecoveryTrace};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChatJobKind {
    Ask,
    Research,
}

pub(super) struct ChatJobExecution {
    pub(super) kind: ChatJobKind,
    pub(super) conversation_id: String,
    pub(super) output_text: String,
    pub(super) low_confidence: bool,
    pub(super) recovery_trace: Option<RecoveryTrace>,
    pub(super) recovery_packet: Option<RecoveryPacket>,
}

pub(super) fn execute_prepared_chat_job<F>(
    core: &CoreService,
    prepared: &PreparedChatRequest,
    mut on_event: F,
) -> Result<ChatJobExecution>
where
    F: FnMut(EventPayload) -> Result<()>,
{
    match &prepared.job {
        PreparedChatJob::Research(job) => {
            let result = core.execute_prepared_research(job, &mut on_event)?;
            Ok(ChatJobExecution {
                kind: ChatJobKind::Research,
                conversation_id: result.conversation_id,
                output_text: result.report_text,
                low_confidence: result.low_confidence,
                recovery_trace: None,
                recovery_packet: None,
            })
        }
        PreparedChatJob::Ask(job) => {
            let result = core.execute_prepared_ask(job, on_event)?;
            Ok(ChatJobExecution {
                kind: ChatJobKind::Ask,
                conversation_id: result.conversation_id,
                output_text: result.output_text,
                low_confidence: result.low_confidence,
                recovery_trace: result.recovery_trace,
                recovery_packet: result.recovery_packet,
            })
        }
    }
}
