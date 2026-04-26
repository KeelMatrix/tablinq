use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use shared_protocol::{
    ArtifactMetadata, ConversationRecoveryManifest, RecoveryManifestFile, RecoveryManifestFileKind,
    RecoveryManifestTurn,
};
use std::fs;
use std::path::Path;

const RECOVERY_MANIFEST_RECENT_TURNS_LIMIT: usize = 12;
const RECOVERY_MANIFEST_ARCHIVED_CONTEXT_LIMIT: usize = 16;
const RECOVERY_MANIFEST_RESPONSE_EXCERPT_LIMIT: usize = 24_000;

pub(super) fn update_conversation_recovery_manifest(
    existing: Option<&ConversationRecoveryManifest>,
    conversation_id: &str,
    request_text: &str,
    response_text: &str,
    files: &[String],
    artifacts: &[ArtifactMetadata],
    completed_at: DateTime<Utc>,
) -> ConversationRecoveryManifest {
    let mut manifest = existing
        .cloned()
        .unwrap_or_else(|| ConversationRecoveryManifest {
            conversation_id: conversation_id.to_owned(),
            archived_context: Vec::new(),
            recent_turns: Vec::new(),
            tracked_input_files: Vec::new(),
            tracked_generated_artifacts: Vec::new(),
            updated_at: completed_at,
        });
    manifest.conversation_id = conversation_id.to_owned();
    manifest.updated_at = completed_at;

    for file in files {
        remember_recovery_manifest_file(
            &mut manifest.tracked_input_files,
            RecoveryManifestFile {
                kind: RecoveryManifestFileKind::InputAttachment,
                file_name: recovery_file_name(file),
                local_path: file.to_owned(),
                content_type: None,
                recorded_at: completed_at,
            },
        );
    }
    for artifact in artifacts {
        remember_recovery_manifest_file(
            &mut manifest.tracked_generated_artifacts,
            RecoveryManifestFile {
                kind: RecoveryManifestFileKind::GeneratedArtifact,
                file_name: artifact.file_name.clone(),
                local_path: artifact.storage_path.clone(),
                content_type: Some(artifact.content_type.clone()),
                recorded_at: completed_at,
            },
        );
    }

    manifest.recent_turns.push(RecoveryManifestTurn {
        completed_at,
        request_text: request_text.trim().to_owned(),
        response_excerpt: preserve_recovery_text(
            response_text,
            RECOVERY_MANIFEST_RESPONSE_EXCERPT_LIMIT,
        ),
        input_files: files.iter().map(|file| recovery_file_name(file)).collect(),
        generated_artifacts: artifacts
            .iter()
            .map(|artifact| artifact.file_name.clone())
            .collect(),
    });

    while manifest.recent_turns.len() > RECOVERY_MANIFEST_RECENT_TURNS_LIMIT {
        let archived = manifest.recent_turns.remove(0);
        manifest
            .archived_context
            .push(summarize_recovery_manifest_turn(&archived));
    }
    while manifest.archived_context.len() > RECOVERY_MANIFEST_ARCHIVED_CONTEXT_LIMIT {
        manifest.archived_context.remove(0);
    }

    manifest
}

pub(super) fn render_provider_generation_recovery_manifest(
    manifest: &ConversationRecoveryManifest,
) -> String {
    let mut content = String::from("# Recovery Context\n\n");
    content.push_str(
        "This attachment contains prior conversation context, file references, and generated artifacts so the current request can continue in a fresh conversation after the previous response failed.\n\n",
    );
    content.push_str(&format!(
        "- Source conversation: {}\n- Updated at: {}\n\n",
        manifest.conversation_id,
        manifest.updated_at.to_rfc3339()
    ));

    if !manifest.archived_context.is_empty() {
        content.push_str("## Earlier Conversation Context\n");
        for line in &manifest.archived_context {
            content.push_str("- ");
            content.push_str(line);
            content.push('\n');
        }
        content.push('\n');
    }

    if !manifest.recent_turns.is_empty() {
        content.push_str("## Recent Conversation Turns\n");
        for (index, turn) in manifest.recent_turns.iter().enumerate() {
            content.push_str(&format!(
                "### Turn {}\n- Completed at: {}\n\n#### User message\n{}\n\n#### Assistant response\n{}\n",
                index + 1,
                turn.completed_at.to_rfc3339(),
                turn.request_text,
                turn.response_excerpt
            ));
            if !turn.input_files.is_empty() {
                content.push_str("- Input files: ");
                content.push_str(&turn.input_files.join(", "));
                content.push('\n');
            }
            if !turn.generated_artifacts.is_empty() {
                content.push_str("- Generated artifacts: ");
                content.push_str(&turn.generated_artifacts.join(", "));
                content.push('\n');
            }
        }
        content.push('\n');
    }

    if !manifest.tracked_input_files.is_empty() {
        content.push_str("## Tracked Input Files\n");
        for file in &manifest.tracked_input_files {
            content.push_str(&format!("- {} ({})\n", file.file_name, file.local_path));
        }
        content.push('\n');
    }

    if !manifest.tracked_generated_artifacts.is_empty() {
        content.push_str("## Tracked Generated Artifacts\n");
        for file in &manifest.tracked_generated_artifacts {
            content.push_str(&format!(
                "- {} ({}){}\n",
                file.file_name,
                file.local_path,
                file.content_type
                    .as_deref()
                    .map(|value| format!(" [{}]", value))
                    .unwrap_or_default()
            ));
        }
    }

    content
}

pub(super) fn write_provider_generation_recovery_manifest_attachment(
    state_root: &Path,
    job_id: &str,
    attempt: u32,
    manifest: &ConversationRecoveryManifest,
) -> Result<String> {
    let dir = state_root.join("recovery-manifests");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let safe_job_id = job_id.replace([':', '/', '\\'], "-");
    let path = dir.join(format!(
        "{safe_job_id}-provider-recovery-attempt-{attempt}.md"
    ));
    fs::write(
        &path,
        render_provider_generation_recovery_manifest(manifest),
    )
    .with_context(|| format!("write {}", path.display()))?;
    Ok(path.display().to_string())
}

pub(super) fn truncate_recovery_text(text: &str, limit: usize) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_owned();
    }

    let mut output = String::new();
    for ch in trimmed.chars().take(limit.saturating_sub(1)) {
        output.push(ch);
    }
    output.push('…');
    output
}

fn preserve_recovery_text(text: &str, limit: usize) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let trimmed = normalized.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_owned();
    }

    let mut output = String::new();
    for ch in trimmed.chars().take(limit.saturating_sub(1)) {
        output.push(ch);
    }
    output.push('…');
    output
}

fn remember_recovery_manifest_file(
    files: &mut Vec<RecoveryManifestFile>,
    entry: RecoveryManifestFile,
) {
    files.retain(|candidate| candidate.local_path != entry.local_path);
    files.insert(0, entry);
}

fn summarize_recovery_manifest_turn(turn: &RecoveryManifestTurn) -> String {
    let mut parts = vec![
        format!(
            "User message: {}",
            truncate_recovery_text(&turn.request_text, 320)
        ),
        format!(
            "Assistant response: {}",
            truncate_recovery_text(&turn.response_excerpt, 1_200)
        ),
    ];
    if !turn.input_files.is_empty() {
        parts.push(format!("Input files: {}", turn.input_files.join(", ")));
    }
    if !turn.generated_artifacts.is_empty() {
        parts.push(format!(
            "Generated artifacts: {}",
            turn.generated_artifacts.join(", ")
        ));
    }
    parts.join(" | ")
}

fn recovery_file_name(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|segment| !segment.trim().is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| path.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        render_provider_generation_recovery_manifest, update_conversation_recovery_manifest,
    };
    use chrono::Utc;
    use shared_protocol::ArtifactMetadata;
    use tempfile::tempdir;

    #[test]
    fn conversation_recovery_manifest_tracks_recent_turns_and_files() {
        let dir = tempdir().unwrap();
        let input_path = dir.path().join("brief.txt");
        let artifact_path = dir.path().join("summary.txt");
        std::fs::write(&input_path, "brief").unwrap();
        std::fs::write(&artifact_path, "summary").unwrap();

        let manifest = update_conversation_recovery_manifest(
            None,
            "conv-1",
            "Summarize the brief",
            "The brief recommends a phased rollout with one generated summary.",
            &[input_path.display().to_string()],
            &[ArtifactMetadata {
                artifact_id: "artifact-1".to_owned(),
                conversation_id: Some("conv-1".to_owned()),
                message_id: Some("msg-1".to_owned()),
                file_name: "summary.txt".to_owned(),
                content_type: "text/plain".to_owned(),
                size_bytes: 7,
                sha256: "sha-1".to_owned(),
                storage_path: artifact_path.display().to_string(),
            }],
            Utc::now(),
        );

        assert_eq!(manifest.recent_turns.len(), 1);
        assert_eq!(
            manifest.recent_turns[0].input_files,
            vec!["brief.txt".to_owned()]
        );
        assert_eq!(
            manifest.recent_turns[0].generated_artifacts,
            vec!["summary.txt".to_owned()]
        );
        assert_eq!(manifest.tracked_input_files.len(), 1);
        assert_eq!(manifest.tracked_generated_artifacts.len(), 1);
    }

    #[test]
    fn provider_generation_recovery_manifest_uses_generic_header_and_keeps_longer_context() {
        let long_response = "detail ".repeat(250);
        let manifest = update_conversation_recovery_manifest(
            None,
            "conv-1",
            "Summarize the long-running task",
            &long_response,
            &[],
            &[],
            Utc::now(),
        );

        assert!(manifest.recent_turns[0].response_excerpt.len() > 1_000);

        let rendered = render_provider_generation_recovery_manifest(&manifest);
        assert!(rendered.starts_with("# Recovery Context"));
        assert!(!rendered.contains("Tablinq"));
        assert!(rendered.contains("#### User message"));
        assert!(rendered.contains("#### Assistant response"));
    }
}
