use super::recovery_manifest::truncate_recovery_text;
use shared_protocol::{
    ConversationRecoveryManifest, ConversationSummary, Effort, ErrorCode, Mode, ModelClass,
    RecoveryPacket, RecoveryStrategy, RecoveryTrace,
};
use std::collections::HashSet;
use std::path::Path;

const PROVIDER_GENERATION_FAILURE_MARKER: &str =
    "something went wrong while generating the response";
const RECOVERY_CONTEXT_SUMMARY_RECENT_TURNS: usize = 3;
const RECOVERY_REPLAY_FILE_LIMIT: usize = 10;
const RECOVERY_REPLAY_HISTORICAL_FILE_LIMIT: usize = 4;

pub(super) fn is_provider_generation_failure_message(message: &str) -> bool {
    message
        .to_ascii_lowercase()
        .contains(PROVIDER_GENERATION_FAILURE_MARKER)
}

pub(super) fn should_auto_recover_provider_generation_failure(
    auto_recover: bool,
    recovery_attempt_count: u32,
) -> bool {
    auto_recover && recovery_attempt_count < 1
}

pub(super) fn provider_recovery_attempt_request_id(job_id: &str, attempt: u32) -> String {
    format!("{job_id}:provider-attempt-{attempt}")
}

pub(super) struct ProviderGenerationRecoveryPacketInput<'a> {
    pub original_input: &'a str,
    pub mode: Mode,
    pub model_class: Option<ModelClass>,
    pub effort: Option<Effort>,
    pub files: &'a [String],
    pub context_summary: Option<String>,
    pub context_manifest: Option<ConversationRecoveryManifest>,
    pub recovery_manifest_path: Option<String>,
    pub replay_files: Vec<String>,
    pub prior_request_id: Option<String>,
    pub prior_conversation_id: Option<String>,
    pub recovery_attempt_count: u32,
}

pub(super) fn build_provider_generation_recovery_packet(
    input: ProviderGenerationRecoveryPacketInput<'_>,
) -> RecoveryPacket {
    RecoveryPacket {
        original_input: input.original_input.to_owned(),
        mode: input.mode,
        model_class: input.model_class,
        effort: input.effort,
        files: input.files.to_vec(),
        context_summary: input.context_summary,
        context_manifest: input.context_manifest,
        recovery_manifest_path: input.recovery_manifest_path,
        replay_files: input.replay_files,
        prior_request_id: input.prior_request_id,
        prior_conversation_id: input.prior_conversation_id,
        failure_code: ErrorCode::ProviderGenerationFailedNonretryable,
        recovery_attempt_count: input.recovery_attempt_count,
    }
}

pub(super) fn build_provider_generation_recovery_trace(
    failure_bundle_id: Option<String>,
    prior_request_id: Option<String>,
    prior_conversation_id: Option<String>,
    recovery_attempt_count: u32,
    recovery_attempted: bool,
) -> RecoveryTrace {
    RecoveryTrace {
        recovery_available: true,
        recovery_attempted,
        recovery_strategy: Some(RecoveryStrategy::ReloadNewConversationReplayWithContext),
        failure_code: Some(ErrorCode::ProviderGenerationFailedNonretryable),
        failure_bundle_id,
        prior_request_id,
        replacement_request_id: None,
        prior_conversation_id,
        replacement_conversation_id: None,
        recovery_attempt_count,
    }
}

pub(super) fn build_provider_generation_context_summary(
    manifest: Option<&ConversationRecoveryManifest>,
    conversation: Option<&ConversationSummary>,
    prior_conversation_id: Option<&str>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(conversation_id) = prior_conversation_id
        && !conversation_id.trim().is_empty()
    {
        parts.push(format!("Prior conversation id: {}", conversation_id.trim()));
    }
    if let Some(manifest) = manifest {
        if !manifest.archived_context.is_empty() {
            parts.push("Earlier preserved context:".to_owned());
            for line in manifest.archived_context.iter().rev().take(2).rev() {
                parts.push(format!("- {}", truncate_recovery_text(line, 220)));
            }
        }

        let recent_turns = manifest
            .recent_turns
            .iter()
            .rev()
            .take(RECOVERY_CONTEXT_SUMMARY_RECENT_TURNS)
            .collect::<Vec<_>>();
        for (index, turn) in recent_turns.into_iter().rev().enumerate() {
            let turn_number = index + 1;
            parts.push(format!(
                "Recent request {turn_number}: {}",
                truncate_recovery_text(&turn.request_text, 220)
            ));
            parts.push(format!(
                "Recent assistant result {turn_number}: {}",
                truncate_recovery_text(&turn.response_excerpt, 320)
            ));
            if !turn.input_files.is_empty() {
                parts.push(format!(
                    "Recent input files {turn_number}: {}",
                    turn.input_files.join(", ")
                ));
            }
            if !turn.generated_artifacts.is_empty() {
                parts.push(format!(
                    "Recent generated artifacts {turn_number}: {}",
                    turn.generated_artifacts.join(", ")
                ));
            }
        }
    } else if let Some(conversation) = conversation {
        if let Some(last_prompt) = conversation.last_prompt.as_deref()
            && !last_prompt.trim().is_empty()
        {
            parts.push(format!(
                "Previous prompt: {}",
                truncate_recovery_text(last_prompt, 240)
            ));
        }
        if let Some(last_response) = conversation.last_response.as_deref()
            && !last_response.trim().is_empty()
        {
            parts.push(format!(
                "Previous assistant response: {}",
                truncate_recovery_text(last_response, 360)
            ));
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

pub(super) fn build_provider_generation_replay_prompt(
    original_input: &str,
    _context_summary: Option<&str>,
    recovery_manifest_attachment: Option<&str>,
) -> String {
    let mut prompt = String::from(
        "Recovery note: The previous response failed during generation. Continue the same task in this new conversation.",
    );
    if let Some(file_name) = recovery_manifest_attachment
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        prompt.push_str("\n\nAttached context file: ");
        prompt.push_str(file_name);
        prompt.push_str(
            ". Use it for prior conversation context, referenced files, and generated artifacts.",
        );
    }
    prompt.push_str(
        "\n\nAnswer this request now, using the attachment for prior context when relevant:\n",
    );
    prompt.push_str(original_input.trim());
    prompt
}

pub(super) fn build_provider_generation_replay_files(
    files: &[String],
    manifest: Option<&ConversationRecoveryManifest>,
    recovery_manifest_path: Option<&str>,
) -> Vec<String> {
    let mut replay_files = Vec::new();
    let mut seen = HashSet::new();
    let mut historical_added = 0usize;

    for file in files {
        push_replay_file(&mut replay_files, &mut seen, file);
    }

    if let Some(path) = recovery_manifest_path {
        push_replay_file(&mut replay_files, &mut seen, path);
    }

    if let Some(manifest) = manifest {
        for file in &manifest.tracked_generated_artifacts {
            if historical_added >= RECOVERY_REPLAY_HISTORICAL_FILE_LIMIT {
                break;
            }
            if !Path::new(&file.local_path).exists() {
                continue;
            }
            let before = replay_files.len();
            push_replay_file(&mut replay_files, &mut seen, &file.local_path);
            if replay_files.len() > before {
                historical_added += 1;
            }
        }
        for file in &manifest.tracked_input_files {
            if historical_added >= RECOVERY_REPLAY_HISTORICAL_FILE_LIMIT {
                break;
            }
            if !Path::new(&file.local_path).exists() {
                continue;
            }
            let before = replay_files.len();
            push_replay_file(&mut replay_files, &mut seen, &file.local_path);
            if replay_files.len() > before {
                historical_added += 1;
            }
        }
    }

    replay_files
}

fn push_replay_file(files: &mut Vec<String>, seen: &mut HashSet<String>, path: &str) {
    let trimmed = path.trim();
    if trimmed.is_empty() || files.len() >= RECOVERY_REPLAY_FILE_LIMIT {
        return;
    }
    if !seen.insert(trimmed.to_owned()) {
        return;
    }
    files.push(trimmed.to_owned());
}

#[cfg(test)]
mod tests {
    use super::{
        build_provider_generation_context_summary, build_provider_generation_replay_files,
        build_provider_generation_replay_prompt, should_auto_recover_provider_generation_failure,
    };
    use crate::core::recovery_manifest::update_conversation_recovery_manifest;
    use chrono::Utc;
    use shared_protocol::{ArtifactMetadata, ConversationSummary};
    use tempfile::tempdir;

    #[test]
    fn provider_generation_context_summary_uses_prior_conversation_snapshot() {
        let summary = build_provider_generation_context_summary(
            None,
            Some(&ConversationSummary {
                conversation_id: "conv-123".to_owned(),
                session_id: "session-1".to_owned(),
                label: "Browser conversation".to_owned(),
                tab_url: Some("https://chatgpt.com/c/conv-123".to_owned()),
                last_prompt: Some("Explain the current system state in detail".to_owned()),
                last_response: Some(
                    "The current system state includes attached files and a partially completed workflow."
                        .to_owned(),
                ),
                last_seen_at: Utc::now(),
            }),
            Some("conv-123"),
        )
        .unwrap();

        assert!(summary.contains("Prior conversation id: conv-123"));
        assert!(summary.contains("Previous prompt: Explain the current system state in detail"));
        assert!(summary.contains(
            "Previous assistant response: The current system state includes attached files"
        ));
    }

    #[test]
    fn provider_generation_replay_prompt_wraps_context_and_original_request() {
        let prompt = build_provider_generation_replay_prompt(
            "reason carefully about this tradeoff",
            Some("Prior conversation id: conv-123\nPrevious assistant response: partial answer"),
            Some("job-1-provider-recovery.md"),
        );

        assert!(prompt.contains("Recovery note: The previous response failed during generation."));
        assert!(prompt.contains("Attached context file: job-1-provider-recovery.md"));
        assert!(!prompt.contains("Preserved context:"));
        assert!(
            prompt.ends_with(
                "Answer this request now, using the attachment for prior context when relevant:\nreason carefully about this tradeoff"
            )
        );
    }

    #[test]
    fn provider_generation_recovery_policy_attempts_once() {
        assert!(should_auto_recover_provider_generation_failure(true, 0));
        assert!(!should_auto_recover_provider_generation_failure(true, 1));
        assert!(!should_auto_recover_provider_generation_failure(false, 0));
    }

    #[test]
    fn provider_generation_context_summary_prefers_durable_manifest() {
        let manifest = update_conversation_recovery_manifest(
            None,
            "conv-123",
            "Explain the deployment history",
            "The deployment history includes an earlier rollback and a generated runbook artifact.",
            &["C:\\inputs\\deploy-notes.md".to_owned()],
            &[],
            Utc::now(),
        );

        let summary = build_provider_generation_context_summary(
            Some(&manifest),
            Some(&ConversationSummary {
                conversation_id: "conv-123".to_owned(),
                session_id: "session-1".to_owned(),
                label: "Browser conversation".to_owned(),
                tab_url: Some("https://chatgpt.com/c/conv-123".to_owned()),
                last_prompt: Some("fallback prompt".to_owned()),
                last_response: Some("fallback response".to_owned()),
                last_seen_at: Utc::now(),
            }),
            Some("conv-123"),
        )
        .unwrap();

        assert!(summary.contains("Recent request 1: Explain the deployment history"));
        assert!(summary.contains("Recent input files 1: deploy-notes.md"));
        assert!(!summary.contains("fallback prompt"));
    }

    #[test]
    fn provider_generation_replay_files_include_manifest_and_historical_context() {
        let dir = tempdir().unwrap();
        let original_input = dir.path().join("latest.txt");
        let prior_input = dir.path().join("brief.txt");
        let artifact = dir.path().join("artifact.txt");
        let manifest_file = dir.path().join("tablinq-recovery.md");
        std::fs::write(&original_input, "latest").unwrap();
        std::fs::write(&prior_input, "brief").unwrap();
        std::fs::write(&artifact, "artifact").unwrap();
        std::fs::write(&manifest_file, "manifest").unwrap();

        let manifest = update_conversation_recovery_manifest(
            None,
            "conv-1",
            "Use the brief",
            "Generated artifact attached.",
            &[prior_input.display().to_string()],
            &[ArtifactMetadata {
                artifact_id: "artifact-1".to_owned(),
                conversation_id: Some("conv-1".to_owned()),
                message_id: Some("msg-1".to_owned()),
                file_name: "artifact.txt".to_owned(),
                content_type: "text/plain".to_owned(),
                size_bytes: 8,
                sha256: "sha-1".to_owned(),
                storage_path: artifact.display().to_string(),
            }],
            Utc::now(),
        );

        let replay_files = build_provider_generation_replay_files(
            &[original_input.display().to_string()],
            Some(&manifest),
            Some(&manifest_file.display().to_string()),
        );

        assert_eq!(replay_files[0], original_input.display().to_string());
        assert!(replay_files.contains(&manifest_file.display().to_string()));
        assert!(replay_files.contains(&artifact.display().to_string()));
        assert!(replay_files.contains(&prior_input.display().to_string()));
    }
}
