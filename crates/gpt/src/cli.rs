use clap::{Args, Parser, Subcommand, ValueEnum};
use shared_protocol::{
    AskCommand, ConversationSelectCommand, Effort, Mode, ModelClass, RateLimitBucket,
    ResearchCommand, ResetRateLimitCommand,
};
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
    Clear(ClearArgs),
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
    RateLimit {
        #[command(subcommand)]
        command: RateLimitCommand,
    },
    Sessions {
        #[command(subcommand)]
        command: SessionsCommand,
    },
    Conversations {
        #[command(subcommand)]
        command: ConversationsCommand,
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
    #[arg(long, conflicts_with = "conversation")]
    pub new_conversation: bool,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct ResearchArgs {
    pub query: String,
    #[arg(long = "file")]
    pub files: Vec<PathBuf>,
    #[arg(long)]
    pub output_dir: Option<PathBuf>,
}

#[derive(Debug, Args, PartialEq, Eq)]
pub struct ClearArgs {
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub force: bool,
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
pub enum RateLimitCommand {
    Reset {
        #[arg(value_enum)]
        bucket: CliRateLimitBucket,
    },
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum SessionsCommand {
    List,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum ConversationsCommand {
    List {
        #[arg(long)]
        json: bool,
    },
    Switch {
        conversation_id: String,
    },
    New {
        #[arg(long)]
        json: bool,
    },
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
    #[command(
        name = "detach",
        alias = "close",
        about = "Detach Tablinq from an attach-first browser session; does not close the user-owned browser"
    )]
    Detach,
    #[command(
        name = "start-debug",
        about = "Start or reuse an external persistent browser debug profile for attach-first mode"
    )]
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

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum CliRateLimitBucket {
    Instant,
    Thinking,
    #[value(name = "file-upload", alias = "native-upload", alias = "upload")]
    FileUpload,
}

pub(crate) fn build_ask_command(args: &AskArgs) -> AskCommand {
    AskCommand {
        prompt: args.prompt.clone(),
        conversation_id: args.conversation.clone(),
        new_conversation: args.new_conversation,
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
        prompt_upload_threshold_chars: None,
        prompt_upload_file_name: None,
        prompt_upload_composer_text: None,
        auto_recover_provider_generation_failure: true,
    }
}

pub(crate) fn build_reset_rate_limit_command(bucket: CliRateLimitBucket) -> ResetRateLimitCommand {
    ResetRateLimitCommand {
        bucket: match bucket {
            CliRateLimitBucket::Instant => RateLimitBucket::Instant,
            CliRateLimitBucket::Thinking => RateLimitBucket::Thinking,
            CliRateLimitBucket::FileUpload => RateLimitBucket::NativeUpload,
        },
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

pub(crate) fn build_conversation_select_command(
    command: &ConversationsCommand,
) -> Option<ConversationSelectCommand> {
    match command {
        ConversationsCommand::List { .. } => None,
        ConversationsCommand::Switch { conversation_id } => Some(ConversationSelectCommand {
            conversation_id: Some(conversation_id.clone()),
            new_conversation: false,
        }),
        ConversationsCommand::New { .. } => Some(ConversationSelectCommand {
            conversation_id: None,
            new_conversation: true,
        }),
    }
}

fn map_mode(value: CliMode) -> Mode {
    match value {
        CliMode::Normal => Mode::Normal,
        CliMode::Search => Mode::Search,
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
    fn rate_limit_reset_command_matches_golden_shape() {
        let cli = parse(&["gpt", "rate-limit", "reset", "file-upload"]);
        assert!(matches!(
            cli.command,
            Command::RateLimit {
                command: RateLimitCommand::Reset {
                    bucket: CliRateLimitBucket::FileUpload
                }
            }
        ));
    }

    #[test]
    fn rate_limit_reset_command_maps_all_supported_buckets() {
        let instant = parse(&["gpt", "rate-limit", "reset", "instant"]);
        let thinking = parse(&["gpt", "rate-limit", "reset", "thinking"]);
        let native_upload_alias = parse(&["gpt", "rate-limit", "reset", "native-upload"]);
        let upload_alias = parse(&["gpt", "rate-limit", "reset", "upload"]);

        let Command::RateLimit {
            command: RateLimitCommand::Reset {
                bucket: instant_bucket,
            },
        } = instant.command
        else {
            panic!("instant command should parse as rate-limit reset");
        };
        let Command::RateLimit {
            command:
                RateLimitCommand::Reset {
                    bucket: thinking_bucket,
                },
        } = thinking.command
        else {
            panic!("thinking command should parse as rate-limit reset");
        };
        let Command::RateLimit {
            command:
                RateLimitCommand::Reset {
                    bucket: native_upload_bucket,
                },
        } = native_upload_alias.command
        else {
            panic!("native-upload command should parse as rate-limit reset");
        };
        let Command::RateLimit {
            command: RateLimitCommand::Reset {
                bucket: upload_bucket,
            },
        } = upload_alias.command
        else {
            panic!("upload command should parse as rate-limit reset");
        };

        assert_eq!(
            build_reset_rate_limit_command(instant_bucket).bucket,
            RateLimitBucket::Instant
        );
        assert_eq!(
            build_reset_rate_limit_command(thinking_bucket).bucket,
            RateLimitBucket::Thinking
        );
        assert_eq!(
            build_reset_rate_limit_command(native_upload_bucket).bucket,
            RateLimitBucket::NativeUpload
        );
        assert_eq!(
            build_reset_rate_limit_command(upload_bucket).bucket,
            RateLimitBucket::NativeUpload
        );
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
    fn conversations_list_command_matches_golden_shape() {
        let cli = parse(&["gpt", "conversations", "list"]);
        assert!(matches!(
            cli.command,
            Command::Conversations {
                command: ConversationsCommand::List { json: false }
            }
        ));
    }

    #[test]
    fn conversations_list_json_command_matches_golden_shape() {
        let cli = parse(&["gpt", "conversations", "list", "--json"]);
        assert!(matches!(
            cli.command,
            Command::Conversations {
                command: ConversationsCommand::List { json: true }
            }
        ));
    }

    #[test]
    fn conversations_switch_command_matches_golden_shape() {
        let cli = parse(&["gpt", "conversations", "switch", "conv-123"]);
        assert!(matches!(
            cli.command,
            Command::Conversations {
                command: ConversationsCommand::Switch { .. }
            }
        ));
    }

    #[test]
    fn conversations_new_command_matches_golden_shape() {
        let cli = parse(&["gpt", "conversations", "new"]);
        assert!(matches!(
            cli.command,
            Command::Conversations {
                command: ConversationsCommand::New { json: false }
            }
        ));
    }

    #[test]
    fn conversations_new_json_command_matches_golden_shape() {
        let cli = parse(&["gpt", "conversations", "new", "--json"]);
        assert!(matches!(
            cli.command,
            Command::Conversations {
                command: ConversationsCommand::New { json: true }
            }
        ));
    }

    #[test]
    fn ask_command_accepts_new_conversation_flag() {
        let cli = parse(&["gpt", "ask", "hello world", "--new-conversation"]);
        assert!(matches!(
            cli.command,
            Command::Ask(AskArgs {
                new_conversation: true,
                ..
            })
        ));
    }

    #[test]
    fn ask_command_rejects_conflicting_conversation_flags() {
        let result = Cli::try_parse_from([
            "gpt",
            "ask",
            "hello world",
            "--conversation",
            "conv-123",
            "--new-conversation",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn ask_command_rejects_research_mode() {
        let result = Cli::try_parse_from(["gpt", "ask", "hello world", "--mode", "research"]);

        assert!(result.is_err());
    }

    #[test]
    fn jobs_show_command_matches_golden_shape() {
        let cli = parse(&["gpt", "jobs", "show", "job-123"]);
        assert!(matches!(cli.command, Command::Jobs { .. }));
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
    fn browser_detach_command_matches_golden_shape() {
        let cli = parse(&["gpt", "browser", "detach"]);
        assert!(matches!(
            cli.command,
            Command::Browser {
                command: BrowserCommand::Detach
            }
        ));
    }

    #[test]
    fn browser_close_alias_matches_detach_command() {
        let cli = parse(&["gpt", "browser", "close"]);
        assert!(matches!(
            cli.command,
            Command::Browser {
                command: BrowserCommand::Detach
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
    fn clear_command_accepts_required_safety_flags() {
        let cli = parse(&["gpt", "clear", "--all", "--force"]);
        assert!(matches!(
            cli.command,
            Command::Clear(ClearArgs {
                all: true,
                force: true
            })
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
            new_conversation: false,
        };

        let command = build_ask_command(&args);
        assert_eq!(command.mode, Mode::Search);
        assert_eq!(command.model_class, Some(ModelClass::Thinking));
        assert_eq!(command.effort, Some(Effort::Extended));
        assert_eq!(command.files, vec!["notes.txt".to_owned()]);
        assert_eq!(command.conversation_id.as_deref(), Some("conv-7"));
        assert!(!command.new_conversation);
        assert!(command.auto_recover_provider_generation_failure);
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
            new_conversation: false,
        };

        let command = build_ask_command(&args);
        assert_eq!(command.model_class, Some(ModelClass::Instant));
        assert!(command.auto_recover_provider_generation_failure);
    }

    #[test]
    fn build_ask_command_maps_new_conversation_flag() {
        let args = AskArgs {
            prompt: "hello".to_owned(),
            mode: CliMode::Normal,
            model_class: None,
            effort: None,
            files: vec![],
            conversation: None,
            new_conversation: true,
        };

        let command = build_ask_command(&args);
        assert!(command.new_conversation);
        assert_eq!(command.conversation_id, None);
    }

    #[test]
    fn build_conversation_select_command_maps_switch_and_new() {
        let switch = build_conversation_select_command(&ConversationsCommand::Switch {
            conversation_id: "conv-9".to_owned(),
        })
        .unwrap();
        assert_eq!(switch.conversation_id.as_deref(), Some("conv-9"));
        assert!(!switch.new_conversation);

        let fresh =
            build_conversation_select_command(&ConversationsCommand::New { json: false }).unwrap();
        assert_eq!(fresh.conversation_id, None);
        assert!(fresh.new_conversation);
    }
}
