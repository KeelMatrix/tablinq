use crate::db::ConversationTurnRecord;
use chrono::Utc;
use shared_protocol::{
    ArtifactSummary, ConversationFileReference, ConversationMessage,
    ConversationMessageExactness, ConversationMessageRole, ConversationMessageSourceKind,
};
use std::collections::BTreeMap;

use super::ConversationInspectionBundle;

#[derive(Clone)]
struct NormalizedMessageGroup {
    sort_at: Option<chrono::DateTime<Utc>>,
    sequence: usize,
    messages: Vec<ConversationMessage>,
}

pub(super) fn build_conversation_messages(
    bundle: &ConversationInspectionBundle,
) -> Vec<ConversationMessage> {
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

fn collapse_ws(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn is_non_empty(value: &str) -> bool {
    !value.trim().is_empty()
}
