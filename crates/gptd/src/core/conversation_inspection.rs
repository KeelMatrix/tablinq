use super::CoreService;
use super::support::build_provider_generation_context_summary;
use crate::db::ConversationTurnRecord;
use anyhow::Result;
use chrono::Utc;
use shared_protocol::{
    ArtifactSummary, ConversationCounts, ConversationDetail, ConversationFileReference,
    ConversationMessage, ConversationMessageExactness, ConversationMessageList,
    ConversationMessageRole, ConversationMessageSourceKind, ConversationRecoveryManifestSummary,
    ConversationRouteHints, ConversationSessionInfo, ConversationSnapshot, ConversationSummary,
};
use std::collections::BTreeMap;

const SNAPSHOT_RECENT_MESSAGE_LIMIT: usize = 8;

#[derive(Clone)]
struct ConversationInspectionBundle {
    conversation: ConversationSummary,
    session: Option<ConversationSessionInfo>,
    manifest: Option<shared_protocol::ConversationRecoveryManifest>,
    turns: Vec<ConversationTurnRecord>,
    artifacts: Vec<ArtifactSummary>,
}

#[derive(Clone)]
struct NormalizedMessageGroup {
    sort_at: Option<chrono::DateTime<Utc>>,
    sequence: usize,
    messages: Vec<ConversationMessage>,
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

fn build_conversation_messages(bundle: &ConversationInspectionBundle) -> Vec<ConversationMessage> {
    let mut groups = Vec::new();
    let mut next_sequence = 0usize;
    let manifest = bundle.manifest.as_ref();
    let input_file_index = manifest_file_index(
        manifest
            .map(|value| value.tracked_input_files.as_slice())
            .unwrap_or(&[]),
    );
    let generated_file_index = manifest_file_index(
        manifest
            .map(|value| value.tracked_generated_artifacts.as_slice())
            .unwrap_or(&[]),
    );
    let matched_manifest_indices = matched_manifest_turns(bundle);

    if let Some(manifest) = manifest {
        for summary in &manifest.archived_context {
            if !is_non_empty(summary) {
                continue;
            }
            groups.push(NormalizedMessageGroup {
                sort_at: None,
                sequence: next_sequence,
                messages: vec![ConversationMessage {
                    source_kind: ConversationMessageSourceKind::SummaryFallback,
                    role: ConversationMessageRole::Summary,
                    exactness: ConversationMessageExactness::Summary,
                    content: summary.trim().to_owned(),
                    created_at: None,
                    request_id: None,
                    job_id: None,
                    message_id: None,
                    artifacts: vec![],
                    input_files: vec![],
                }],
            });
            next_sequence += 1;
        }
    }

    for (index, turn) in bundle.turns.iter().enumerate() {
        let mut messages = Vec::new();
        let matched_manifest_turn = matched_manifest_indices
            .get(index)
            .and_then(|value| *value)
            .and_then(|manifest_index| {
                bundle
                    .manifest
                    .as_ref()
                    .and_then(|manifest| manifest.recent_turns.get(manifest_index))
            });
        let request_id = turn
            .request
            .as_ref()
            .map(|request| request.request_id.clone());
        let job_id = Some(turn.job.job_id.clone());
        let user_created_at = turn
            .request
            .as_ref()
            .map(|request| request.start_time)
            .or(Some(turn.job.created_at));
        let assistant_created_at = turn
            .request
            .as_ref()
            .and_then(|request| request.end_time)
            .or(Some(turn.job.updated_at))
            .or_else(|| matched_manifest_turn.map(|manifest_turn| manifest_turn.completed_at));
        let input_files = matched_manifest_turn
            .map(|manifest_turn| {
                manifest_file_references(&manifest_turn.input_files, &input_file_index)
            })
            .unwrap_or_default();

        if let Some(content) = turn
            .job
            .input_text
            .as_deref()
            .filter(|value| is_non_empty(value))
        {
            messages.push(ConversationMessage {
                source_kind: ConversationMessageSourceKind::RequestDetail,
                role: ConversationMessageRole::User,
                exactness: ConversationMessageExactness::Exact,
                content: content.trim().to_owned(),
                created_at: user_created_at,
                request_id: request_id.clone(),
                job_id: job_id.clone(),
                message_id: None,
                artifacts: vec![],
                input_files: input_files.clone(),
            });
        } else if let Some(manifest_turn) = matched_manifest_turn
            && is_non_empty(&manifest_turn.request_text)
        {
            messages.push(ConversationMessage {
                source_kind: ConversationMessageSourceKind::RecoveryManifest,
                role: ConversationMessageRole::User,
                exactness: ConversationMessageExactness::Exact,
                content: manifest_turn.request_text.trim().to_owned(),
                created_at: user_created_at.or(Some(manifest_turn.completed_at)),
                request_id: request_id.clone(),
                job_id: job_id.clone(),
                message_id: None,
                artifacts: vec![],
                input_files: input_files.clone(),
            });
        }

        let artifact_references = if !turn.job.artifacts.is_empty() {
            turn.job
                .artifacts
                .iter()
                .map(file_reference_from_artifact)
                .collect::<Vec<_>>()
        } else {
            matched_manifest_turn
                .map(|manifest_turn| {
                    manifest_file_references(
                        &manifest_turn.generated_artifacts,
                        &generated_file_index,
                    )
                })
                .unwrap_or_default()
        };
        let message_id = stable_message_id(&artifact_references);

        if let Some(content) = turn
            .job
            .output_text
            .as_deref()
            .filter(|value| is_non_empty(value))
        {
            messages.push(ConversationMessage {
                source_kind: ConversationMessageSourceKind::RequestDetail,
                role: ConversationMessageRole::Assistant,
                exactness: ConversationMessageExactness::Exact,
                content: content.trim().to_owned(),
                created_at: assistant_created_at,
                request_id: request_id.clone(),
                job_id: job_id.clone(),
                message_id: message_id.clone(),
                artifacts: artifact_references.clone(),
                input_files: vec![],
            });
        } else if let Some(manifest_turn) = matched_manifest_turn
            && is_non_empty(&manifest_turn.response_excerpt)
        {
            messages.push(ConversationMessage {
                source_kind: ConversationMessageSourceKind::RecoveryManifest,
                role: ConversationMessageRole::Assistant,
                exactness: ConversationMessageExactness::Excerpt,
                content: manifest_turn.response_excerpt.trim().to_owned(),
                created_at: assistant_created_at.or(Some(manifest_turn.completed_at)),
                request_id: request_id.clone(),
                job_id,
                message_id,
                artifacts: artifact_references,
                input_files: vec![],
            });
        }

        if !messages.is_empty() {
            groups.push(NormalizedMessageGroup {
                sort_at: user_created_at.or(assistant_created_at),
                sequence: next_sequence,
                messages,
            });
            next_sequence += 1;
        }
    }

    if let Some(manifest) = manifest {
        for (manifest_index, manifest_turn) in manifest.recent_turns.iter().enumerate() {
            if matched_manifest_indices
                .iter()
                .flatten()
                .any(|value| *value == manifest_index)
            {
                continue;
            }
            let mut messages = Vec::new();
            let input_files =
                manifest_file_references(&manifest_turn.input_files, &input_file_index);
            if is_non_empty(&manifest_turn.request_text) {
                messages.push(ConversationMessage {
                    source_kind: ConversationMessageSourceKind::RecoveryManifest,
                    role: ConversationMessageRole::User,
                    exactness: ConversationMessageExactness::Exact,
                    content: manifest_turn.request_text.trim().to_owned(),
                    created_at: Some(manifest_turn.completed_at),
                    request_id: None,
                    job_id: None,
                    message_id: None,
                    artifacts: vec![],
                    input_files: input_files.clone(),
                });
            }
            let artifacts =
                manifest_file_references(&manifest_turn.generated_artifacts, &generated_file_index);
            if is_non_empty(&manifest_turn.response_excerpt) {
                messages.push(ConversationMessage {
                    source_kind: ConversationMessageSourceKind::RecoveryManifest,
                    role: ConversationMessageRole::Assistant,
                    exactness: ConversationMessageExactness::Excerpt,
                    content: manifest_turn.response_excerpt.trim().to_owned(),
                    created_at: Some(manifest_turn.completed_at),
                    request_id: None,
                    job_id: None,
                    message_id: stable_message_id(&artifacts),
                    artifacts,
                    input_files: vec![],
                });
            }
            if !messages.is_empty() {
                groups.push(NormalizedMessageGroup {
                    sort_at: Some(manifest_turn.completed_at),
                    sequence: next_sequence,
                    messages,
                });
                next_sequence += 1;
            }
        }
    }

    if groups.is_empty() {
        if let Some(last_prompt) = bundle
            .conversation
            .last_prompt
            .as_deref()
            .filter(|value| is_non_empty(value))
        {
            groups.push(NormalizedMessageGroup {
                sort_at: Some(bundle.conversation.last_seen_at),
                sequence: next_sequence,
                messages: vec![ConversationMessage {
                    source_kind: ConversationMessageSourceKind::SummaryFallback,
                    role: ConversationMessageRole::Summary,
                    exactness: ConversationMessageExactness::Summary,
                    content: format!("Previous prompt summary: {}", last_prompt.trim()),
                    created_at: Some(bundle.conversation.last_seen_at),
                    request_id: None,
                    job_id: None,
                    message_id: None,
                    artifacts: vec![],
                    input_files: vec![],
                }],
            });
            next_sequence += 1;
        }
        if let Some(last_response) = bundle
            .conversation
            .last_response
            .as_deref()
            .filter(|value| is_non_empty(value))
        {
            groups.push(NormalizedMessageGroup {
                sort_at: Some(bundle.conversation.last_seen_at),
                sequence: next_sequence,
                messages: vec![ConversationMessage {
                    source_kind: ConversationMessageSourceKind::SummaryFallback,
                    role: ConversationMessageRole::Summary,
                    exactness: ConversationMessageExactness::Summary,
                    content: format!("Previous assistant summary: {}", last_response.trim()),
                    created_at: Some(bundle.conversation.last_seen_at),
                    request_id: None,
                    job_id: None,
                    message_id: None,
                    artifacts: vec![],
                    input_files: vec![],
                }],
            });
        }
    }

    groups.sort_by(|left, right| match (left.sort_at, right.sort_at) {
        (None, None) => left.sequence.cmp(&right.sequence),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (Some(left_at), Some(right_at)) => left_at
            .cmp(&right_at)
            .then_with(|| left.sequence.cmp(&right.sequence)),
    });

    groups
        .into_iter()
        .flat_map(|group| group.messages)
        .collect::<Vec<_>>()
}

fn matched_manifest_turns(bundle: &ConversationInspectionBundle) -> Vec<Option<usize>> {
    let Some(manifest) = bundle.manifest.as_ref() else {
        return vec![None; bundle.turns.len()];
    };
    let mut matches = vec![None; bundle.turns.len()];
    let mut used = vec![false; manifest.recent_turns.len()];
    for turn_index in (0..bundle.turns.len()).rev() {
        if let Some(manifest_index) = (0..manifest.recent_turns.len()).rev().find(|candidate| {
            !used[*candidate]
                && turn_matches_manifest(
                    &bundle.turns[turn_index],
                    &manifest.recent_turns[*candidate],
                )
        }) {
            used[manifest_index] = true;
            matches[turn_index] = Some(manifest_index);
        }
    }
    matches
}

fn turn_matches_manifest(
    turn: &ConversationTurnRecord,
    manifest_turn: &shared_protocol::RecoveryManifestTurn,
) -> bool {
    if let Some(input_text) = turn
        .job
        .input_text
        .as_deref()
        .filter(|value| is_non_empty(value))
        && collapse_ws(input_text) == collapse_ws(&manifest_turn.request_text)
    {
        return true;
    }
    if let Some(output_text) = turn
        .job
        .output_text
        .as_deref()
        .filter(|value| is_non_empty(value))
    {
        let excerpt = manifest_turn.response_excerpt.trim().trim_end_matches('…');
        if !excerpt.is_empty() && collapse_ws(output_text).starts_with(&collapse_ws(excerpt)) {
            return true;
        }
    }
    false
}

fn manifest_file_index(
    files: &[shared_protocol::RecoveryManifestFile],
) -> BTreeMap<String, shared_protocol::RecoveryManifestFile> {
    let mut index = BTreeMap::new();
    for file in files {
        index.insert(file.file_name.clone(), file.clone());
    }
    index
}

fn manifest_file_references(
    file_names: &[String],
    index: &BTreeMap<String, shared_protocol::RecoveryManifestFile>,
) -> Vec<ConversationFileReference> {
    file_names
        .iter()
        .map(|file_name| {
            index
                .get(file_name)
                .map(file_reference_from_manifest_file)
                .unwrap_or_else(|| ConversationFileReference {
                    file_name: file_name.clone(),
                    artifact_id: None,
                    local_path: None,
                    content_type: None,
                    size_bytes: None,
                    sha256: None,
                    storage_path: None,
                    message_id: None,
                    created_at: None,
                })
        })
        .collect()
}

fn file_reference_from_artifact(artifact: &ArtifactSummary) -> ConversationFileReference {
    ConversationFileReference {
        file_name: artifact.file_name.clone(),
        artifact_id: Some(artifact.artifact_id.clone()),
        local_path: None,
        content_type: Some(artifact.content_type.clone()),
        size_bytes: Some(artifact.size_bytes),
        sha256: Some(artifact.sha256.clone()),
        storage_path: Some(artifact.storage_path.clone()),
        message_id: artifact.message_id.clone(),
        created_at: Some(artifact.created_at),
    }
}

fn file_reference_from_manifest_file(
    file: &shared_protocol::RecoveryManifestFile,
) -> ConversationFileReference {
    ConversationFileReference {
        file_name: file.file_name.clone(),
        artifact_id: None,
        local_path: Some(file.local_path.clone()),
        content_type: file.content_type.clone(),
        size_bytes: None,
        sha256: None,
        storage_path: Some(file.local_path.clone()),
        message_id: None,
        created_at: Some(file.recorded_at),
    }
}

fn stable_message_id(files: &[ConversationFileReference]) -> Option<String> {
    let mut message_id = None;
    for file in files {
        let candidate = file
            .message_id
            .as_deref()
            .filter(|value| !value.trim().is_empty());
        match (message_id.as_deref(), candidate) {
            (None, Some(value)) => message_id = Some(value.to_owned()),
            (Some(existing), Some(value)) if existing == value => {}
            (Some(_), Some(_)) => return None,
            (_, None) => {}
        }
    }
    message_id
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

fn collapse_ws(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}
