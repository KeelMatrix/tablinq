use super::{PreparedChatRequest, RequestOutcome, RequestOutcomeKind};
use crate::http::RequestContext;
use anyhow::Result;
use chrono::{DateTime, Utc};
use gptd::CoreService;
use gptd::logging::JsonLogger;
use serde_json::json;
use shared_protocol::{RecoveryPacket, RecoveryTrace};

const STREAM_DISCONNECT_PHASE: &str = "before streaming completed";
const FINAL_RESPONSE_DISCONNECT_PHASE: &str = "before the final response was written";
const DOWNSTREAM_RECOVERY_REASON: &str =
    "gateway client disconnected, but the upstream request completed";

#[derive(Debug, Clone)]
pub(super) struct DownstreamInterruption {
    pub(super) interrupted_at: DateTime<Utc>,
    pub(super) reason: String,
}

pub(super) struct DownstreamRecovery {
    pub(super) conversation_id: Option<String>,
    pub(super) recovery_trace: Option<RecoveryTrace>,
    pub(super) recovery_packet: Option<RecoveryPacket>,
}

pub(super) fn stream_disconnect_message(err: &anyhow::Error) -> String {
    client_disconnect_message(err, STREAM_DISCONNECT_PHASE)
}

pub(super) fn final_response_disconnect_message(err: &anyhow::Error) -> String {
    client_disconnect_message(err, FINAL_RESPONSE_DISCONNECT_PHASE)
}

pub(super) fn mark_downstream_interrupted(
    core: &CoreService,
    prepared: &PreparedChatRequest,
    conversation_id: Option<&str>,
    reason: String,
) -> DownstreamInterruption {
    let interrupted_at = Utc::now();
    let _ = core.mark_job_interrupted(prepared.job_id(), conversation_id, &reason);
    DownstreamInterruption {
        interrupted_at,
        reason,
    }
}

pub(super) fn recover_downstream_interruption(
    core: &CoreService,
    logger: Option<&JsonLogger>,
    context: &RequestContext,
    prepared: &PreparedChatRequest,
    interruption: DownstreamInterruption,
    recovery: DownstreamRecovery,
) -> Result<RequestOutcome> {
    let recovered_at = Utc::now();
    let recovery_reason = DOWNSTREAM_RECOVERY_REASON.to_owned();
    let _ = core.mark_job_recovered(
        prepared.job_id(),
        recovery.conversation_id.as_deref(),
        &recovery_reason,
    );
    if let Some(logger) = logger {
        logger.info(
            "gateway recovered interrupted client stream",
            json!({
                "request_id": context.request_id,
                "job_id": prepared.job_id(),
            }),
        )?;
    }
    Ok(RequestOutcome {
        kind: RequestOutcomeKind::Recovered,
        low_confidence: true,
        conversation_id: recovery.conversation_id,
        interrupted_at: Some(interruption.interrupted_at),
        interruption_reason: Some(interruption.reason),
        recovered_at: Some(recovered_at),
        recovery_reason: Some(recovery_reason),
        failed_at: None,
        failure_reason: None,
        recovery_trace: recovery.recovery_trace,
        recovery_packet: recovery.recovery_packet,
    })
}

fn client_disconnect_message(err: &anyhow::Error, phase: &str) -> String {
    format!("gateway client disconnected {phase}: {err:#}")
}
