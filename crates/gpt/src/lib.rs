use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Parser, PartialEq, Eq)]
#[command(name = "gpt", version, about = "Tablinq CLI scaffold")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum Command {
    Ask(AskArgs),
    Research(ResearchArgs),
    Capabilities,
    Doctor,
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
    Open,
    Close,
    ResetProfile,
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

pub fn render_command_summary(cli: &Cli) -> String {
    match &cli.command {
        Command::Ask(args) => format!(
            "command=ask\nprompt={}\nmode={}\nmodel_class={}\neffort={}\nfiles={}\nconversation={}",
            args.prompt,
            render_mode(args.mode),
            args.model_class
                .map(render_model)
                .unwrap_or("unspecified".to_owned()),
            args.effort
                .map(render_effort)
                .unwrap_or("unspecified".to_owned()),
            render_paths(&args.files),
            args.conversation.as_deref().unwrap_or("<new>")
        ),
        Command::Research(args) => format!(
            "command=research\nquery={}\nfiles={}\noutput_dir={}",
            args.query,
            render_paths(&args.files),
            args.output_dir
                .as_ref()
                .map(|value| value.display().to_string())
                .unwrap_or_else(|| "<default>".to_owned())
        ),
        Command::Capabilities => "command=capabilities".to_owned(),
        Command::Doctor => "command=doctor".to_owned(),
        Command::Daemon { command } => {
            format!("command=daemon\nsubcommand={}", render_daemon(command))
        }
        Command::Config { command } => {
            format!("command=config\nsubcommand={}", render_config(command))
        }
        Command::Sessions { command } => {
            format!("command=sessions\nsubcommand={}", render_sessions(command))
        }
        Command::Jobs { command } => format!("command=jobs\n{}", render_jobs(command)),
        Command::Artifacts { command } => {
            format!("command=artifacts\n{}", render_artifacts(command))
        }
        Command::Browser { command } => {
            format!("command=browser\nsubcommand={}", render_browser(command))
        }
    }
}

fn render_mode(value: CliMode) -> String {
    match value {
        CliMode::Normal => "normal",
        CliMode::Search => "search",
        CliMode::Research => "research",
    }
    .to_owned()
}

fn render_model(value: CliModelClass) -> String {
    match value {
        CliModelClass::Instant => "instant",
        CliModelClass::Thinking => "thinking",
    }
    .to_owned()
}

fn render_effort(value: CliEffort) -> String {
    match value {
        CliEffort::None => "none",
        CliEffort::Standard => "standard",
        CliEffort::Extended => "extended",
    }
    .to_owned()
}

fn render_daemon(value: &DaemonCommand) -> &'static str {
    match value {
        DaemonCommand::Start => "start",
        DaemonCommand::Status => "status",
        DaemonCommand::Stop => "stop",
    }
}

fn render_config(value: &ConfigCommand) -> &'static str {
    match value {
        ConfigCommand::Show => "show",
    }
}

fn render_sessions(value: &SessionsCommand) -> &'static str {
    match value {
        SessionsCommand::List => "list",
    }
}

fn render_jobs(value: &JobsCommand) -> String {
    match value {
        JobsCommand::List => "subcommand=list".to_owned(),
        JobsCommand::Show { job_id } => format!("subcommand=show\njob_id={job_id}"),
        JobsCommand::Cancel { job_id } => format!("subcommand=cancel\njob_id={job_id}"),
    }
}

fn render_artifacts(value: &ArtifactsCommand) -> String {
    match value {
        ArtifactsCommand::List => "subcommand=list".to_owned(),
        ArtifactsCommand::Pull {
            artifact_id,
            destination_dir,
        } => format!(
            "subcommand=pull\nartifact_id={artifact_id}\ndestination_dir={}",
            destination_dir.display()
        ),
    }
}

fn render_browser(value: &BrowserCommand) -> &'static str {
    match value {
        BrowserCommand::Open => "open",
        BrowserCommand::Close => "close",
        BrowserCommand::ResetProfile => "reset-profile",
    }
}

fn render_paths(values: &[PathBuf]) -> String {
    if values.is_empty() {
        "<none>".to_owned()
    } else {
        values
            .iter()
            .map(|value| value.display().to_string())
            .collect::<Vec<_>>()
            .join(",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        assert_eq!(
            render_command_summary(&cli),
            "command=ask\nprompt=hello world\nmode=search\nmodel_class=thinking\neffort=standard\nfiles=docs/spec.pdf,fixtures/input.csv\nconversation=conv-123"
        );
    }

    #[test]
    fn daemon_status_command_matches_golden_shape() {
        let cli = parse(&["gpt", "daemon", "status"]);

        assert_eq!(
            render_command_summary(&cli),
            "command=daemon\nsubcommand=status"
        );
    }

    #[test]
    fn artifact_pull_command_matches_golden_shape() {
        let cli = parse(&["gpt", "artifacts", "pull", "last", "--to", "."]);

        assert_eq!(
            render_command_summary(&cli),
            "command=artifacts\nsubcommand=pull\nartifact_id=last\ndestination_dir=."
        );
    }
}
