use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::json;
use shared_protocol::{
    AskCommand, BrowserOpenRequest, CancelJobCommand, DaemonRequest, DaemonResponse, Effort,
    EventPayload, Mode, ModelClass, PullArtifactCommand, ResearchCommand, StreamChunkKind,
    build_state_paths, resolve_state_root,
};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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
    Open {
        #[arg(long)]
        target_url: Option<String>,
        #[arg(long)]
        browser_path: Option<PathBuf>,
        #[arg(long, hide = true)]
        headless: bool,
    },
    Close,
    ResetProfile,
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

pub fn run(cli: &Cli) -> Result<String> {
    match &cli.command {
        Command::Daemon { command } => match command {
            DaemonCommand::Start => start_daemon(cli.state_root.as_deref()),
            DaemonCommand::Status => {
                send_daemon_request(cli.state_root.as_deref(), DaemonRequest::Status)
            }
            DaemonCommand::Stop => {
                send_daemon_request(cli.state_root.as_deref(), DaemonRequest::Shutdown)
            }
        },
        Command::Doctor => send_daemon_request(cli.state_root.as_deref(), DaemonRequest::Doctor),
        Command::Config { command } => match command {
            ConfigCommand::Show => config_show(cli.state_root.as_deref()),
        },
        Command::Sessions { command } => match command {
            SessionsCommand::List => {
                send_daemon_request(cli.state_root.as_deref(), DaemonRequest::SessionsList)
            }
        },
        Command::Jobs { command } => match command {
            JobsCommand::List => {
                send_daemon_request(cli.state_root.as_deref(), DaemonRequest::JobsList)
            }
            JobsCommand::Show { job_id } => send_daemon_request(
                cli.state_root.as_deref(),
                DaemonRequest::JobsShow(job_id.clone()),
            ),
            JobsCommand::Cancel { job_id } => send_daemon_request(
                cli.state_root.as_deref(),
                DaemonRequest::JobsCancel(CancelJobCommand {
                    job_id: job_id.clone(),
                }),
            ),
        },
        Command::Ask(args) => run_ask(cli.state_root.as_deref(), args),
        Command::Research(args) => run_research(cli.state_root.as_deref(), args),
        Command::Capabilities => {
            send_daemon_request(cli.state_root.as_deref(), DaemonRequest::Capabilities)
        }
        Command::Artifacts { command } => run_artifacts_command(cli.state_root.as_deref(), command),
        Command::Browser { command } => run_browser_command(cli.state_root.as_deref(), command),
    }
}

fn run_artifacts_command(state_root: Option<&Path>, command: &ArtifactsCommand) -> Result<String> {
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

fn run_browser_command(state_root: Option<&Path>, command: &BrowserCommand) -> Result<String> {
    match command {
        BrowserCommand::Open {
            target_url,
            browser_path,
            headless,
        } => send_daemon_request(
            state_root,
            DaemonRequest::BrowserOpen(BrowserOpenRequest {
                target_url: target_url.clone(),
                browser_path: browser_path.as_ref().map(|path| path.display().to_string()),
                headless: Some(*headless),
            }),
        ),
        BrowserCommand::Close => send_daemon_request(state_root, DaemonRequest::BrowserClose),
        BrowserCommand::ResetProfile => {
            send_daemon_request(state_root, DaemonRequest::BrowserResetProfile)
        }
        BrowserCommand::ProfileInfo => {
            send_daemon_request(state_root, DaemonRequest::BrowserProfileInfo)
        }
    }
}

fn send_daemon_request(state_root: Option<&Path>, request: DaemonRequest) -> Result<String> {
    let paths = build_state_paths(resolve_state_root(state_root));
    let response = gptd::send_request(&paths.ipc_endpoint, &request)
        .with_context(|| format!("connect to daemon at {}", paths.ipc_endpoint))?;
    serde_json::to_string_pretty(&response).context("serialize daemon response")
}

fn run_ask(state_root: Option<&Path>, args: &AskArgs) -> Result<String> {
    let paths = build_state_paths(resolve_state_root(state_root));
    let command = AskCommand {
        prompt: args.prompt.clone(),
        conversation_id: args.conversation.clone(),
        mode: map_mode(args.mode),
        model_class: args.model_class.map(map_model),
        effort: args.effort.map(map_effort),
        files: args
            .files
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
    };

    let mut stdout = io::stdout();
    let mut printed_any = false;
    let response =
        gptd::send_request_stream(&paths.ipc_endpoint, &DaemonRequest::Ask(command), |event| {
            if let EventPayload::StreamChunk(chunk) = event {
                if chunk.kind == StreamChunkKind::Delta && !chunk.text.is_empty() {
                    write!(stdout, "{}", chunk.text)?;
                    stdout.flush()?;
                    printed_any = true;
                }
                if chunk.kind == StreamChunkKind::Done && printed_any {
                    writeln!(stdout)?;
                    stdout.flush()?;
                }
            }
            Ok(())
        })
        .with_context(|| format!("connect to daemon at {}", paths.ipc_endpoint))?;

    match response {
        DaemonResponse::Ask(result) => {
            if !printed_any && !result.output_text.is_empty() {
                writeln!(stdout, "{}", result.output_text)?;
            }
            Ok(String::new())
        }
        DaemonResponse::Error(error) => bail!("{}", error.message),
        other => bail!("unexpected ask response: {other:?}"),
    }
}

fn run_research(state_root: Option<&Path>, args: &ResearchArgs) -> Result<String> {
    let paths = build_state_paths(resolve_state_root(state_root));
    let command = ResearchCommand {
        query: args.query.clone(),
        files: args
            .files
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        output_dir: args
            .output_dir
            .as_ref()
            .map(|path| path.display().to_string()),
    };

    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let mut printed_any = false;
    let mut last_status: Option<JobStatusLine> = None;
    let response = gptd::send_request_stream(
        &paths.ipc_endpoint,
        &DaemonRequest::Research(command),
        |event| {
            match event {
                EventPayload::StreamChunk(chunk) => match chunk.kind {
                    StreamChunkKind::Delta if !chunk.text.is_empty() => {
                        write!(stdout, "{}", chunk.text)?;
                        stdout.flush()?;
                        printed_any = true;
                    }
                    StreamChunkKind::Done if printed_any => {
                        writeln!(stdout)?;
                        stdout.flush()?;
                    }
                    _ => {}
                },
                EventPayload::JobState(state) => {
                    let next = JobStatusLine {
                        status: format!("{:?}", state.status).to_lowercase(),
                        message: state.message.unwrap_or_default(),
                    };
                    if last_status.as_ref() != Some(&next) {
                        writeln!(stderr, "[{}] {}", next.status, next.message)?;
                        stderr.flush()?;
                        last_status = Some(next);
                    }
                }
                _ => {}
            }
            Ok(())
        },
    )
    .with_context(|| format!("connect to daemon at {}", paths.ipc_endpoint))?;

    match response {
        DaemonResponse::Research(result) => {
            if !printed_any && !result.report_text.is_empty() {
                writeln!(stdout, "{}", result.report_text)?;
            }
            Ok(String::new())
        }
        DaemonResponse::Error(error) => bail!("{}", error.message),
        other => bail!("unexpected research response: {other:?}"),
    }
}

fn config_show(state_root: Option<&Path>) -> Result<String> {
    let config = gptd::AppConfig::load_or_create(state_root)?;
    serde_json::to_string_pretty(&config.snapshot()).context("serialize config snapshot")
}

fn start_daemon(state_root: Option<&Path>) -> Result<String> {
    let resolved_state_root = resolve_state_root(state_root);
    let paths = build_state_paths(&resolved_state_root);

    if gptd::daemon_is_responsive(&paths.ipc_endpoint) {
        return Ok(json!({
            "status": "already_running",
            "ipc_endpoint": paths.ipc_endpoint,
        })
        .to_string());
    }

    let daemon_bin = resolve_gptd_binary();
    let mut command = ProcessCommand::new(&daemon_bin);
    command
        .arg("serve")
        .arg("--state-root")
        .arg(&resolved_state_root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }

    let mut child = command
        .spawn()
        .with_context(|| format!("spawn daemon binary {}", daemon_bin.display()))?;

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if gptd::daemon_is_responsive(&paths.ipc_endpoint) {
            return Ok(json!({
                "status": "started",
                "ipc_endpoint": paths.ipc_endpoint,
                "state_root": resolved_state_root.display().to_string(),
            })
            .to_string());
        }

        if let Some(status) = child.try_wait().context("poll daemon startup")? {
            bail!("daemon exited during startup with status {status}");
        }

        if Instant::now() >= deadline {
            bail!("daemon did not become ready within 5 seconds");
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn resolve_gptd_binary() -> PathBuf {
    if let Ok(value) = std::env::var("TABLINQ_GPTD_BIN") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    let current = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("gpt"));
    let mut sibling = current.clone();
    sibling.set_file_name(if cfg!(windows) { "gptd.exe" } else { "gptd" });
    if sibling.exists() {
        sibling
    } else {
        PathBuf::from(if cfg!(windows) { "gptd.exe" } else { "gptd" })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct JobStatusLine {
    status: String,
    message: String,
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

        assert_eq!(cli.state_root, None);
        assert!(matches!(cli.command, Command::Ask(_)));
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
}
