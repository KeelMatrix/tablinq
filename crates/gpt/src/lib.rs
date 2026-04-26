mod browser;
mod build_info;
mod cli;
mod daemon;
mod status;
mod terminal;

use anyhow::{Context, Result};
use serde_json::Value;
use shared_protocol::{CancelJobCommand, DaemonRequest, PullArtifactCommand};
use std::path::Path;

use crate::browser::run_browser_command;
use crate::daemon::{send_daemon_request, send_daemon_response, start_daemon};
use crate::status::{config_show, operational_report};
use crate::terminal::{
    execute_ask_with_terminal_presentation, execute_research_with_terminal_presentation,
    format_conversation_list, format_conversation_selection,
};

pub use crate::cli::{
    ArtifactsCommand, AskArgs, BrowserCommand, Cli, CliEffort, CliMode, CliModelClass, Command,
    ConfigCommand, ConversationsCommand, DaemonCommand, GatewayCommand, JobsCommand, ResearchArgs,
    SessionsCommand, VersionArgs,
};
pub use crate::terminal::format_cli_error;

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
                    render_conversation_new(cli.state_root.as_deref(), request)
                }
            }
            ConversationsCommand::Switch { .. } => Ok(CliExecution::Json(send_daemon_request(
                cli.state_root.as_deref(),
                DaemonRequest::ConversationSelect(
                    crate::cli::build_conversation_select_command(command)
                        .expect("non-list conversation command"),
                ),
            )?)),
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

fn render_conversation_new(
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
