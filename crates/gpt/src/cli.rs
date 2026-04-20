use clap::{Args, Parser, Subcommand, ValueEnum};
use shared_protocol::{AskCommand, Effort, Mode, ModelClass, ResearchCommand};
use std::path::PathBuf;

#[derive(Debug, Parser, PartialEq, Eq)]
#[command(name = "gpt", version, about = "Tablinq local CLI")]
pub struct Cli {
    #[arg(long, global = true)]
    pub state_root: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Command {
    Version(VersionArgs),
    Status,
    Ask(AskArgs),
    Research(ResearchArgs),
    Capabilities,
    Doctor,
    Gateway {
        #[command(subcommand)]
        command: GatewayCommand,
    },
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Sessions {
        #[command(subcommand)]
        command: SessionsCommand,
    },
    Jobs {
        #[command(subcommand)]
        command: JobsCommand,
    },
    Artifacts {
        #[command(subcommand)]
        command: ArtifactsCommand,
    },
    Browser {
        #[command(subcommand)]
        command: BrowserCommand,
    },
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct VersionArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct AskArgs {
    pub prompt: String,
    #[arg(long, value_enum, default_value_t = CliMode::Normal)]
    pub mode: CliMode,
    #[arg(long = "model", value_enum)]
    pub model_class: Option<CliModelClass>,
    #[arg(long, value_enum)]
    pub effort: Option<CliEffort>,
    #[arg(long = "file")]
    pub files: Vec<PathBuf>,
    #[arg(long)]
    pub conversation: Option<String>,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct ResearchArgs {
    pub query: String,
    #[arg(long = "file")]
    pub files: Vec<PathBuf>,
    #[arg(long)]
    pub output_dir: Option<PathBuf>,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum DaemonCommand {
    Start,
    Status,
    Stop,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum GatewayCommand {
    Start,
    Status,
    Stop,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum ConfigCommand {
    Show,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum SessionsCommand {
    List,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum JobsCommand {
    List,
    Show { job_id: String },
    Cancel { job_id: String },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum ArtifactsCommand {
    List,
    Pull {
        artifact_id: String,
        #[arg(long = "to")]
        destination_dir: PathBuf,
    },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum BrowserCommand {
    Open {
        #[arg(long)]
        target_url: Option<String>,
        #[arg(long)]
        browser_path: Option<PathBuf>,
        #[arg(long, hide = true)]
        headless: bool,
    },
    Attach {
        #[arg(long)]
        target_url: Option<String>,
        #[arg(long)]
        browser_path: Option<PathBuf>,
        #[arg(long, hide = true)]
        headless: bool,
    },
    Close,
    Logout,
    #[command(name = "switch-account")]
    SwitchAccount,
    ResetProfile,
    #[command(name = "start-debug")]
    StartDebug {
        #[arg(long)]
        target_url: Option<String>,
        #[arg(long)]
        browser_path: Option<PathBuf>,
    },
    #[command(name = "connection-info")]
    ConnectionInfo,
    ProfileInfo,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum CliMode {
    Normal,
    Search,
    Research,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum CliModelClass {
    Instant,
    Thinking,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum CliEffort {
    None,
    Standard,
    Extended,
}

pub(crate) fn build_ask_command(args: &AskArgs) -> AskCommand {
    AskCommand {
        prompt: args.prompt.clone(),
        conversation_id: args.conversation.clone(),
        new_conversation: false,
        mode: map_mode(args.mode),
        model_class: Some(
            args.model_class
                .map(map_model)
                .unwrap_or(ModelClass::Instant),
        ),
        effort: args.effort.map(map_effort),
        files: args
            .files
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
    }
}

pub(crate) fn build_research_command(args: &ResearchArgs) -> ResearchCommand {
    ResearchCommand {
        query: args.query.clone(),
        conversation_id: None,
        new_conversation: false,
        files: args
            .files
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        output_dir: args
            .output_dir
            .as_ref()
            .map(|path| path.display().to_string()),
    }
}

fn map_mode(value: CliMode) -> Mode {
    match value {
        CliMode::Normal => Mode::Normal,
        CliMode::Search => Mode::Search,
        CliMode::Research => Mode::Research,
    }
}

fn map_model(value: CliModelClass) -> ModelClass {
    match value {
        CliModelClass::Instant => ModelClass::Instant,
        CliModelClass::Thinking => ModelClass::Thinking,
    }
}

fn map_effort(value: CliEffort) -> Effort {
    match value {
        CliEffort::None => Effort::None,
        CliEffort::Standard => Effort::Standard,
        CliEffort::Extended => Effort::Extended,
    }
}

pub(crate) fn resolve_ask_timeout_ms(config: &gptd::AppConfig, effort: Option<Effort>) -> u64 {
    match effort {
        Some(Effort::Extended) => config.extended_thinking_timeout_ms(),
        _ => config.ask_timeout_ms(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        Cli::parse_from(args)
    }

    #[test]
    fn ask_command_matches_golden_shape() {
        let cli = parse(&[
            "gpt",
            "ask",
            "hello world",
            "--mode",
            "search",
            "--model",
            "thinking",
            "--effort",
            "standard",
            "--file",
            "docs/spec.pdf",
            "--file",
            "fixtures/input.csv",
            "--conversation",
            "conv-123",
        ]);

        assert_eq!(cli.state_root, None);
        assert!(matches!(cli.command, Command::Ask(_)));
    }

    #[test]
    fn version_command_matches_golden_shape() {
        let cli = parse(&["gpt", "version", "--json"]);
        assert!(matches!(
            cli.command,
            Command::Version(VersionArgs { json: true })
        ));
    }

    #[test]
    fn status_command_matches_golden_shape() {
        let cli = parse(&["gpt", "status"]);
        assert!(matches!(cli.command, Command::Status));
    }

    #[test]
    fn daemon_status_command_matches_golden_shape() {
        let cli = parse(&["gpt", "--state-root", "tmp/state", "daemon", "status"]);

        assert_eq!(cli.state_root, Some(PathBuf::from("tmp/state")));
        assert!(matches!(
            cli.command,
            Command::Daemon {
                command: DaemonCommand::Status
            }
        ));
    }

    #[test]
    fn gateway_status_command_matches_golden_shape() {
        let cli = parse(&["gpt", "gateway", "status"]);
        assert!(matches!(
            cli.command,
            Command::Gateway {
                command: GatewayCommand::Status
            }
        ));
    }

    #[test]
    fn artifact_pull_command_matches_golden_shape() {
        let cli = parse(&["gpt", "artifacts", "pull", "last", "--to", "."]);
        assert!(matches!(cli.command, Command::Artifacts { .. }));
    }

    #[test]
    fn research_command_matches_golden_shape() {
        let cli = parse(&[
            "gpt",
            "research",
            "latest roadmap",
            "--file",
            "notes.txt",
            "--output-dir",
            "out",
        ]);

        assert!(matches!(cli.command, Command::Research(_)));
    }

    #[test]
    fn jobs_show_command_matches_golden_shape() {
        let cli = parse(&["gpt", "jobs", "show", "job-123"]);
        assert!(matches!(cli.command, Command::Jobs { .. }));
    }

    #[test]
    fn browser_logout_command_matches_golden_shape() {
        let cli = parse(&["gpt", "browser", "logout"]);
        assert!(matches!(
            cli.command,
            Command::Browser {
                command: BrowserCommand::Logout
            }
        ));
    }

    #[test]
    fn browser_switch_account_command_matches_golden_shape() {
        let cli = parse(&["gpt", "browser", "switch-account"]);
        assert!(matches!(
            cli.command,
            Command::Browser {
                command: BrowserCommand::SwitchAccount
            }
        ));
    }

    #[test]
    fn browser_attach_command_matches_golden_shape() {
        let cli = parse(&[
            "gpt",
            "browser",
            "attach",
            "--target-url",
            "https://chatgpt.com/",
        ]);
        assert!(matches!(
            cli.command,
            Command::Browser {
                command: BrowserCommand::Attach { .. }
            }
        ));
    }

    #[test]
    fn browser_start_debug_command_matches_golden_shape() {
        let cli = parse(&[
            "gpt",
            "browser",
            "start-debug",
            "--target-url",
            "https://chatgpt.com/",
        ]);
        assert!(matches!(
            cli.command,
            Command::Browser {
                command: BrowserCommand::StartDebug { .. }
            }
        ));
    }

    #[test]
    fn browser_connection_info_command_matches_golden_shape() {
        let cli = parse(&["gpt", "browser", "connection-info"]);
        assert!(matches!(
            cli.command,
            Command::Browser {
                command: BrowserCommand::ConnectionInfo
            }
        ));
    }

    #[test]
    fn build_ask_command_maps_cli_flags_into_protocol_command() {
        let args = AskArgs {
            prompt: "hello".to_owned(),
            mode: CliMode::Search,
            model_class: Some(CliModelClass::Thinking),
            effort: Some(CliEffort::Extended),
            files: vec![PathBuf::from("notes.txt")],
            conversation: Some("conv-7".to_owned()),
        };

        let command = build_ask_command(&args);
        assert_eq!(command.mode, Mode::Search);
        assert_eq!(command.model_class, Some(ModelClass::Thinking));
        assert_eq!(command.effort, Some(Effort::Extended));
        assert_eq!(command.files, vec!["notes.txt".to_owned()]);
        assert_eq!(command.conversation_id.as_deref(), Some("conv-7"));
    }

    #[test]
    fn build_ask_command_defaults_to_instant_model() {
        let args = AskArgs {
            prompt: "hello".to_owned(),
            mode: CliMode::Normal,
            model_class: None,
            effort: None,
            files: vec![],
            conversation: None,
        };

        let command = build_ask_command(&args);
        assert_eq!(command.model_class, Some(ModelClass::Instant));
    }
}
