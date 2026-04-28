use super::CoreService;
use super::recovery::build_provider_generation_context_summary;
use crate::db::ConversationTurnRecord;
use anyhow::Result;
use shared_protocol::{
    ArtifactSummary, ConversationCounts, ConversationDetail, ConversationMessage,
    ConversationMessageList, ConversationRecoveryManifestSummary, ConversationRouteHints,
    ConversationSessionInfo, ConversationSnapshot, ConversationSummary,
};

mod normalize;

use normalize::{build_conversation_messages, is_non_empty};

const SNAPSHOT_RECENT_MESSAGE_LIMIT: usize = 8;

#[derive(Clone)]
pub(super) struct ConversationInspectionBundle {
    pub(super) conversation: ConversationSummary,
    pub(super) session: Option<ConversationSessionInfo>,
    pub(super) manifest: Option<shared_protocol::ConversationRecoveryManifest>,
    pub(super) turns: Vec<ConversationTurnRecord>,
    pub(super) artifacts: Vec<ArtifactSummary>,
}

impl CoreService {
    pub fn find_conversation_detail(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationDetail>> {
        let Some(bundle) = self.load_conversation_inspection_bundle(conversation_id)? else {
            return Ok(None);
        };
        let messages = build_conversation_messages(&bundle);
        Ok(Some(build_conversation_detail(&bundle, messages.len())))
    }

    pub fn list_conversation_history(
        &self,
        conversation_id: &str,
        limit: usize,
        offset: usize,
    ) -> Result<Option<ConversationMessageList>> {
        let Some(bundle) = self.load_conversation_inspection_bundle(conversation_id)? else {
            return Ok(None);
        };
        let messages = build_conversation_messages(&bundle);
        let total_items = messages.len();
        let start = offset.min(total_items);
        let end = start.saturating_add(limit).min(total_items);
        Ok(Some(ConversationMessageList {
            conversation_id: bundle.conversation.conversation_id.clone(),
            messages: messages[start..end].to_vec(),
            limit,
            offset,
            next_offset: (end < total_items).then_some(end),
            total_items,
        }))
    }

    pub fn conversation_snapshot(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationSnapshot>> {
        let Some(bundle) = self.load_conversation_inspection_bundle(conversation_id)? else {
            return Ok(None);
        };
        let messages = build_conversation_messages(&bundle);
        let detail = build_conversation_detail(&bundle, messages.len());
        let recent_messages = if messages.len() > SNAPSHOT_RECENT_MESSAGE_LIMIT {
            messages[messages.len() - SNAPSHOT_RECENT_MESSAGE_LIMIT..].to_vec()
        } else {
            messages.clone()
        };
        let latest_request_id = bundle.turns.iter().rev().find_map(|turn| {
            turn.request
                .as_ref()
                .map(|request| request.request_id.clone())
        });
        let latest_job_id = bundle.turns.last().map(|turn| turn.job.job_id.clone());
        let latest_recovery_turn = bundle.turns.iter().rev().find(|turn| {
            turn.request.as_ref().is_some_and(|request| {
                request.recovery_packet.is_some() || request.recovery_trace.is_some()
            }) || turn.job.recovery_packet.is_some()
                || turn.job.recovery_trace.is_some()
        });
        let latest_recovery_packet = latest_recovery_turn.and_then(|turn| {
            turn.request
                .as_ref()
                .and_then(|request| request.recovery_packet.clone())
                .or_else(|| turn.job.recovery_packet.clone())
        });
        let latest_recovery_trace = latest_recovery_turn.and_then(|turn| {
            turn.request
                .as_ref()
                .and_then(|request| request.recovery_trace.clone())
                .or_else(|| turn.job.recovery_trace.clone())
        });
        let context_summary = latest_recovery_packet
            .as_ref()
            .and_then(|packet| packet.context_summary.clone())
            .or_else(|| {
                build_provider_generation_context_summary(
                    bundle.manifest.as_ref(),
                    Some(&bundle.conversation),
                    Some(bundle.conversation.conversation_id.as_str()),
                )
            });
        let (suitable_for_fresh_conversation_replay, replay_readiness_reason) =
            conversation_replay_readiness(
                &bundle,
                &context_summary,
                &messages,
                latest_recovery_packet.as_ref(),
            );

        Ok(Some(ConversationSnapshot {
            conversation: detail,
            context_summary,
            recovery_manifest: bundle.manifest.clone(),
            recent_messages,
            tracked_input_files: bundle
                .manifest
                .as_ref()
                .map(|manifest| manifest.tracked_input_files.clone())
                .unwrap_or_default(),
            tracked_generated_artifacts: bundle
                .manifest
                .as_ref()
                .map(|manifest| manifest.tracked_generated_artifacts.clone())
                .unwrap_or_default(),
            latest_recovery_packet,
            latest_recovery_trace: latest_recovery_trace.clone(),
            latest_request_id,
            latest_job_id,
            replacement_request_id: latest_recovery_trace
                .as_ref()
                .and_then(|trace| trace.replacement_request_id.clone()),
            replacement_conversation_id: latest_recovery_trace
                .as_ref()
                .and_then(|trace| trace.replacement_conversation_id.clone()),
            suitable_for_fresh_conversation_replay,
            replay_readiness_reason,
        }))
    }

    fn load_conversation_inspection_bundle(
        &self,
        conversation_id: &str,
    ) -> Result<Option<ConversationInspectionBundle>> {
        let db = self.open_db()?;
        let Some(conversation) = db.find_conversation(conversation_id)? else {
            return Ok(None);
        };
        Ok(Some(ConversationInspectionBundle {
            session: db.find_session_info(&conversation.session_id)?,
            manifest: db.conversation_recovery_manifest(conversation_id)?,
            turns: db.list_conversation_turn_records(conversation_id)?,
            artifacts: db.list_conversation_artifacts(conversation_id)?,
            conversation,
        }))
    }
}

fn build_conversation_detail(
    bundle: &ConversationInspectionBundle,
    history_item_count: usize,
) -> ConversationDetail {
    let latest_request_id = bundle.turns.iter().rev().find_map(|turn| {
        turn.request
            .as_ref()
            .map(|request| request.request_id.clone())
    });
    let latest_job_id = bundle.turns.last().map(|turn| turn.job.job_id.clone());
    ConversationDetail {
        summary: bundle.conversation.clone(),
        session: bundle.session.clone(),
        latest_request_id: latest_request_id.clone(),
        latest_job_id,
        recovery_manifest: bundle.manifest.as_ref().map(|manifest| {
            ConversationRecoveryManifestSummary {
                updated_at: manifest.updated_at,
                archived_context_count: manifest.archived_context.len(),
                recent_turn_count: manifest.recent_turns.len(),
                tracked_input_file_count: manifest.tracked_input_files.len(),
                tracked_generated_artifact_count: manifest.tracked_generated_artifacts.len(),
            }
        }),
        counts: ConversationCounts {
            request_count: bundle
                .turns
                .iter()
                .filter(|turn| turn.request.is_some())
                .count(),
            artifact_count: bundle.artifacts.len(),
            history_item_count,
        },
        routes: conversation_route_hints(
            &bundle.conversation.conversation_id,
            latest_request_id.as_deref(),
        ),
    }
}

fn conversation_route_hints(
    conversation_id: &str,
    latest_request_id: Option<&str>,
) -> ConversationRouteHints {
    ConversationRouteHints {
        detail: format!("/v1/conversations/{conversation_id}"),
        messages: format!("/v1/conversations/{conversation_id}/messages"),
        snapshot: format!("/v1/conversations/{conversation_id}/snapshot"),
        select: "/v1/conversations/select".to_owned(),
        latest_request: latest_request_id.map(|request_id| format!("/v1/requests/{request_id}")),
        latest_request_evidence: latest_request_id
            .map(|request_id| format!("/v1/requests/{request_id}/evidence")),
    }
}

fn conversation_replay_readiness(
    bundle: &ConversationInspectionBundle,
    context_summary: &Option<String>,
    messages: &[ConversationMessage],
    latest_recovery_packet: Option<&shared_protocol::RecoveryPacket>,
) -> (bool, String) {
    if latest_recovery_packet.is_some() {
        return (
            true,
            "latest recovery packet preserves replay context for a fresh conversation".to_owned(),
        );
    }
    if let Some(manifest) = bundle.manifest.as_ref()
        && (!manifest.recent_turns.is_empty()
            || !manifest.archived_context.is_empty()
            || !manifest.tracked_input_files.is_empty()
            || !manifest.tracked_generated_artifacts.is_empty())
    {
        return (
            true,
            "durable conversation recovery manifest is available for replay-safe continuation"
                .to_owned(),
        );
    }
    if context_summary.as_deref().is_some_and(is_non_empty) || !messages.is_empty() {
        return (
            true,
            "snapshot relies on persisted normalized history and summary fallback rather than a recovery packet".to_owned(),
        );
    }
    (
        false,
        "no durable replay context is available beyond conversation metadata".to_owned(),
    )
}
