mod browser;
mod build_info;
mod cli;
mod daemon;
mod status;
mod terminal;

use anyhow::{Context, Result};
use serde_json::Value;
use shared_protocol::{CancelJobCommand, ClearStateResult, DaemonRequest, PullArtifactCommand};
use std::path::Path;

use crate::browser::run_browser_command;
use crate::daemon::{send_daemon_request, send_daemon_response, start_daemon};
use crate::status::{config_show, operational_report};
use crate::terminal::{
    execute_ask_with_terminal_presentation, execute_research_with_terminal_presentation,
    format_conversation_list, format_conversation_selection,
};

pub use crate::cli::{
    ArtifactsCommand, AskArgs, BrowserCommand, ClearArgs, Cli, CliEffort, CliMode, CliModelClass,
    CliRateLimitBucket, Command, ConfigCommand, ConversationsCommand, DaemonCommand,
    GatewayCommand, JobsCommand, RateLimitCommand, ResearchArgs, SessionsCommand, VersionArgs,
};
pub use crate::terminal::format_cli_error;

const CLEAR_SAFETY_MESSAGE: &str =
    "Refusing to clear Tablinq state. Re-run as: gpt clear --all --force";

pub fn run(cli: &Cli) -> Result<String> {
    render_execution(execute(cli)?)
}

#[derive(Debug)]
enum CliExecution {
    Text(String),
    Json(Value),
    Printed,
}

fn execute(cli: &Cli) -> Result<CliExecution> {
    match &cli.command {
        Command::Version(args) => Ok(CliExecution::Text(build_info::render(args.json))),
        Command::Status => Ok(CliExecution::Json(operational_report(
            cli.state_root.as_deref(),
        )?)),
        Command::Daemon { command } => match command {
            DaemonCommand::Start => {
                Ok(CliExecution::Text(start_daemon(cli.state_root.as_deref())?))
            }
            DaemonCommand::Status => Ok(CliExecution::Json(send_daemon_request(
                cli.state_root.as_deref(),
                DaemonRequest::Status,
            )?)),
            DaemonCommand::Stop => Ok(CliExecution::Json(send_daemon_request(
                cli.state_root.as_deref(),
                DaemonRequest::Shutdown,
            )?)),
        },
        Command::Doctor => Ok(CliExecution::Json(operational_report(
            cli.state_root.as_deref(),
        )?)),
        Command::Gateway { command } => match command {
            GatewayCommand::Start => Ok(CliExecution::Json(
                tablinq_gateway::start_gateway_process(cli.state_root.as_deref())?,
            )),
            GatewayCommand::Status => Ok(CliExecution::Json(tablinq_gateway::gateway_status(
                cli.state_root.as_deref(),
            )?)),
            GatewayCommand::Stop => Ok(CliExecution::Json(tablinq_gateway::stop_gateway_process(
                cli.state_root.as_deref(),
            )?)),
        },
        Command::Config { command } => match command {
            ConfigCommand::Show => Ok(CliExecution::Json(config_show(cli.state_root.as_deref())?)),
        },
        Command::RateLimit { command } => match command {
            RateLimitCommand::Reset { bucket } => {
                run_rate_limit_reset_command(cli.state_root.as_deref(), *bucket)
            }
        },
        Command::Sessions { command } => match command {
            SessionsCommand::List => Ok(CliExecution::Json(send_daemon_request(
                cli.state_root.as_deref(),
                DaemonRequest::SessionsList,
            )?)),
        },
        Command::Conversations { command } => match command {
            ConversationsCommand::List { json } => {
                if *json {
                    Ok(CliExecution::Json(send_daemon_request(
                        cli.state_root.as_deref(),
                        DaemonRequest::ConversationsList,
                    )?))
                } else {
                    render_conversations_list(cli.state_root.as_deref())
                }
            }
            ConversationsCommand::New { json } => {
                let request = DaemonRequest::ConversationSelect(
                    crate::cli::build_conversation_select_command(command)
                        .expect("non-list conversation command"),
                );
                if *json {
                    Ok(CliExecution::Json(send_daemon_request(
                        cli.state_root.as_deref(),
                        request,
                    )?))
                } else {
                    render_conversation_selection(cli.state_root.as_deref(), request)
                }
            }
            ConversationsCommand::Switch { .. } => render_conversation_selection(
                cli.state_root.as_deref(),
                DaemonRequest::ConversationSelect(
                    crate::cli::build_conversation_select_command(command)
                        .expect("non-list conversation command"),
                ),
            ),
        },
        Command::Jobs { command } => match command {
            JobsCommand::List => Ok(CliExecution::Json(send_daemon_request(
                cli.state_root.as_deref(),
                DaemonRequest::JobsList,
            )?)),
            JobsCommand::Show { job_id } => Ok(CliExecution::Json(send_daemon_request(
                cli.state_root.as_deref(),
                DaemonRequest::JobsShow(job_id.clone()),
            )?)),
            JobsCommand::Cancel { job_id } => Ok(CliExecution::Json(send_daemon_request(
                cli.state_root.as_deref(),
                DaemonRequest::JobsCancel(CancelJobCommand {
                    job_id: job_id.clone(),
                }),
            )?)),
        },
        Command::Ask(args) => {
            execute_ask_with_terminal_presentation(cli.state_root.as_deref(), args)?;
            Ok(CliExecution::Printed)
        }
        Command::Research(args) => {
            execute_research_with_terminal_presentation(cli.state_root.as_deref(), args)?;
            Ok(CliExecution::Printed)
        }
        Command::Clear(args) => run_clear_command(cli.state_root.as_deref(), args),
        Command::Capabilities => Ok(CliExecution::Json(send_daemon_request(
            cli.state_root.as_deref(),
            DaemonRequest::Capabilities,
        )?)),
        Command::Artifacts { command } => Ok(CliExecution::Json(run_artifacts_command(
            cli.state_root.as_deref(),
            command,
        )?)),
        Command::Browser { command } => Ok(CliExecution::Json(run_browser_command(
            cli.state_root.as_deref(),
            command,
        )?)),
    }
}

fn run_rate_limit_reset_command(
    state_root: Option<&Path>,
    bucket: CliRateLimitBucket,
) -> Result<CliExecution> {
    let request = DaemonRequest::ResetRateLimit(crate::cli::build_reset_rate_limit_command(bucket));
    let response = send_daemon_response(state_root, request)?;
    if let shared_protocol::DaemonResponse::Error(error) = &response {
        anyhow::bail!("{}", error.message);
    }
    let shared_protocol::DaemonResponse::ResetRateLimit(result) = response else {
        anyhow::bail!("unexpected rate-limit reset response: {response:?}");
    };
    Ok(CliExecution::Text(format!(
        "Reset {:?} rate limit: {} attempts deleted.",
        result.bucket, result.deleted_attempts
    )))
}

fn render_conversations_list(state_root: Option<&Path>) -> Result<CliExecution> {
    let response = send_daemon_response(state_root, DaemonRequest::ConversationsList)?;
    if let shared_protocol::DaemonResponse::Error(error) = &response {
        anyhow::bail!("{}", error.message);
    }
    let shared_protocol::DaemonResponse::Conversations(list) = response else {
        anyhow::bail!("unexpected conversations list response: {response:?}");
    };
    let active_conversation_id = send_daemon_response(state_root, DaemonRequest::SessionsList)
        .ok()
        .and_then(|response| match response {
            shared_protocol::DaemonResponse::Sessions(sessions) => sessions
                .sessions
                .first()
                .and_then(|session| session.latest_conversation_id.clone()),
            _ => None,
        });
    Ok(CliExecution::Text(format_conversation_list(
        &list,
        active_conversation_id.as_deref(),
    )))
}

fn render_conversation_selection(
    state_root: Option<&Path>,
    request: DaemonRequest,
) -> Result<CliExecution> {
    let response = send_daemon_response(state_root, request)?;
    if let shared_protocol::DaemonResponse::Error(error) = &response {
        anyhow::bail!("{}", error.message);
    }
    let shared_protocol::DaemonResponse::ConversationSelection(selection) = response else {
        anyhow::bail!("unexpected conversation selection response: {response:?}");
    };
    Ok(CliExecution::Text(format_conversation_selection(
        &selection,
    )))
}

fn run_clear_command(state_root: Option<&Path>, args: &ClearArgs) -> Result<CliExecution> {
    if !(args.all && args.force) {
        anyhow::bail!(CLEAR_SAFETY_MESSAGE);
    }

    let response = send_daemon_response(state_root, DaemonRequest::ClearAllState)?;
    if let shared_protocol::DaemonResponse::Error(error) = &response {
        anyhow::bail!("{}", error.message);
    }
    let shared_protocol::DaemonResponse::ClearState(result) = response else {
        anyhow::bail!("unexpected clear response: {response:?}");
    };
    Ok(CliExecution::Text(format_clear_result(&result)))
}

fn format_clear_result(result: &ClearStateResult) -> String {
    format!(
        "Cleared Tablinq state: {} database rows, {} artifact files deleted, {} missing, {} failed.",
        result.database_rows_deleted(),
        result.artifact_files_deleted,
        result.artifact_files_missing,
        result.artifact_files_failed
    )
}

pub(crate) fn render_execution(execution: CliExecution) -> Result<String> {
    match execution {
        CliExecution::Text(text) => Ok(text),
        CliExecution::Json(value) => {
            serde_json::to_string_pretty(&value).context("serialize CLI JSON output")
        }
        CliExecution::Printed => Ok(String::new()),
    }
}

fn run_artifacts_command(state_root: Option<&Path>, command: &ArtifactsCommand) -> Result<Value> {
    match command {
        ArtifactsCommand::List => send_daemon_request(state_root, DaemonRequest::ArtifactsList),
        ArtifactsCommand::Pull {
            artifact_id,
            destination_dir,
        } => send_daemon_request(
            state_root,
            DaemonRequest::ArtifactPull(PullArtifactCommand {
                artifact_id: artifact_id.clone(),
                destination_dir: destination_dir.display().to_string(),
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use clap::Parser;
    use shared_protocol::{ArtifactMetadata, JobKind, JobStatus};
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn build_rate_limit_reset_maps_file_upload_bucket() {
        let command = crate::cli::build_reset_rate_limit_command(CliRateLimitBucket::FileUpload);
        assert_eq!(
            command.bucket,
            shared_protocol::RateLimitBucket::NativeUpload
        );
    }

    #[test]
    fn clear_command_refuses_without_all_and_force() {
        let cli = Cli::parse_from(["gpt", "clear", "--all"]);

        let err = run(&cli).unwrap_err();

        assert!(format!("{err:#}").contains("gpt clear --all --force"));
    }

    #[test]
    fn clear_all_force_clears_sessions_conversations_jobs_artifacts_and_files() {
        let dir = tempdir().unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();
        let artifact_path = dir.path().join("artifact.txt");
        std::fs::write(&artifact_path, "artifact").unwrap();
        seed_clear_fixture(&config, artifact_path.display().to_string());
        let daemon = start_test_daemon(config.clone());

        let cli = Cli::parse_from([
            "gpt",
            "--state-root",
            dir.path().to_str().unwrap(),
            "clear",
            "--all",
            "--force",
        ]);
        let output = run(&cli).unwrap();

        assert!(output.contains("Cleared Tablinq state"));
        assert!(output.contains("1 artifact files deleted"));
        assert!(!artifact_path.exists());
        let db = gptd::Database::open(&config.paths.db_path).unwrap();
        assert!(db.list_sessions().unwrap().sessions.is_empty());
        assert!(db.list_conversations().unwrap().conversations.is_empty());
        assert!(db.list_jobs().unwrap().jobs.is_empty());
        assert!(db.list_artifacts().unwrap().artifacts.is_empty());

        stop_test_daemon(&config, daemon);
    }

    fn seed_clear_fixture(config: &gptd::AppConfig, artifact_path: String) {
        let db = gptd::Database::open(&config.paths.db_path).unwrap();
        let now = Utc::now();
        db.upsert_session(
            "session-clear",
            "Session",
            Some("https://chatgpt.com/"),
            now,
        )
        .unwrap();
        db.upsert_conversation(
            "conv-clear",
            "session-clear",
            "Conversation",
            Some("https://chatgpt.com/c/conv-clear"),
            Some("prompt"),
            Some("response"),
            None,
            now,
        )
        .unwrap();
        db.insert_job(
            "job-clear",
            JobStatus::Completed,
            Some("complete"),
            Some("conv-clear"),
            now,
            now,
        )
        .unwrap();
        db.upsert_job_detail("job-clear", JobKind::Ask, Some("prompt"), None)
            .unwrap();
        db.insert_artifact(
            &ArtifactMetadata {
                artifact_id: "artifact-clear".to_owned(),
                conversation_id: Some("conv-clear".to_owned()),
                message_id: Some("msg-clear".to_owned()),
                file_name: "artifact.txt".to_owned(),
                content_type: "text/plain".to_owned(),
                size_bytes: 8,
                sha256: "sha-clear".to_owned(),
                storage_path: artifact_path,
            },
            now,
        )
        .unwrap();
        db.link_job_artifact("job-clear", "artifact-clear").unwrap();
    }

    fn start_test_daemon(config: gptd::AppConfig) -> thread::JoinHandle<anyhow::Result<()>> {
        let endpoint = config.paths.ipc_endpoint.clone();
        let service_config = config.clone();
        let handle = thread::spawn(move || gptd::run_service(service_config));
        for _ in 0..50 {
            if gptd::daemon_is_responsive(&endpoint) {
                return handle;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("test daemon did not start at {endpoint}");
    }

    fn stop_test_daemon(config: &gptd::AppConfig, handle: thread::JoinHandle<anyhow::Result<()>>) {
        let _ = gptd::send_request(&config.paths.ipc_endpoint, &DaemonRequest::Shutdown);
        handle.join().unwrap().unwrap();
    }
}
