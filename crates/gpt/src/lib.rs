mod build_info;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use shared_protocol::{
    AskCommand, BrowserConnectionMode, BrowserOpenRequest, CancelJobCommand, DaemonRequest,
    DaemonResponse, Effort, EventPayload, Mode, ModelClass, PullArtifactCommand, ResearchCommand,
    StreamChunkKind, build_state_paths, resolve_state_root,
};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const IPC_TIMEOUT_BUFFER: Duration = Duration::from_secs(30);

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

pub fn run(cli: &Cli) -> Result<String> {
    render_execution(execute(cli)?)
}

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

fn render_execution(execution: CliExecution) -> Result<String> {
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

fn run_browser_command(state_root: Option<&Path>, command: &BrowserCommand) -> Result<Value> {
    let config = gptd::AppConfig::load_or_create(state_root)?;
    match command {
        BrowserCommand::Open {
            target_url,
            browser_path,
            headless,
        }
        | BrowserCommand::Attach {
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
        BrowserCommand::Logout | BrowserCommand::SwitchAccount => {
            send_daemon_request(state_root, DaemonRequest::BrowserLogout)
        }
        BrowserCommand::ResetProfile => {
            if !supports_managed_profile_reset(config.browser_connection_mode()) {
                bail!(
                    "browser reset-profile is only available in managed_fallback or managed_only; current mode is attach_first"
                );
            }
            send_daemon_request(state_root, DaemonRequest::BrowserResetProfile)
        }
        BrowserCommand::ConnectionInfo => browser_connection_info(state_root, &config),
        BrowserCommand::ProfileInfo => {
            send_daemon_request(state_root, DaemonRequest::BrowserProfileInfo)
        }
    }
}

fn browser_connection_info(state_root: Option<&Path>, config: &gptd::AppConfig) -> Result<Value> {
    let browser = if gptd::daemon_is_responsive(&config.paths.ipc_endpoint) {
        send_daemon_request(state_root, DaemonRequest::BrowserProfileInfo)?
    } else {
        Value::Null
    };
    let mode = config.browser_connection_mode();

    Ok(json!({
        "attach_mode": mode,
        "debug_endpoint": config.browser_debug_endpoint(),
        "debug_host": config.browser_debug_host(),
        "debug_port": config.browser_debug_port(),
        "browser_channel": config.browser_channel(),
        "browser_path": config.browser_path(),
        "target_url": config.browser_target_url(),
        "extension_required": config.browser_extension_required(),
        "auto_open_target_tab": config.browser_auto_open_target_tab(),
        "recommended_commands": browser_recovery_commands(mode),
        "browser": browser,
    }))
}

fn send_daemon_request(state_root: Option<&Path>, request: DaemonRequest) -> Result<Value> {
    let config = gptd::AppConfig::load_or_create(state_root)?;
    let paths = build_state_paths(resolve_state_root(state_root));
    let timeout = Duration::from_millis(config.gateway_request_timeout_ms());
    let endpoint = paths.ipc_endpoint.clone();
    let response = run_with_timeout(
        timeout,
        format!("daemon request timed out after {} ms", timeout.as_millis()),
        move || {
            gptd::send_request(&endpoint, &request)
                .with_context(|| format!("connect to daemon at {endpoint}"))
        },
    )?;
    serde_json::to_value(response).context("serialize daemon response")
}

fn execute_ask_with_terminal_presentation(state_root: Option<&Path>, args: &AskArgs) -> Result<()> {
    let config = gptd::AppConfig::load_or_create(state_root)?;
    let paths = build_state_paths(resolve_state_root(state_root));
    let command = build_ask_command(args);
    let timeout = Duration::from_millis(config.ask_timeout_ms()) + IPC_TIMEOUT_BUFFER;
    let endpoint = paths.ipc_endpoint.clone();
    let (tx, rx) = mpsc::channel();

    let mut stdout = io::stdout();
    let mut printed_any = false;
    thread::spawn(move || {
        let request = DaemonRequest::Ask(command);
        let result = gptd::send_request_stream(&endpoint, &request, |event| {
            tx.send(CliStreamMessage::Event(Box::new(event)))
                .map_err(|_| anyhow::anyhow!("CLI stream receiver closed"))?;
            Ok(())
        })
        .with_context(|| format!("connect to daemon at {endpoint}"));
        let _ = tx.send(CliStreamMessage::Done(Box::new(result)));
    });

    let response = loop {
        match rx.recv_timeout(timeout) {
            Ok(CliStreamMessage::Event(event)) => {
                if let EventPayload::StreamChunk(chunk) = *event {
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
            }
            Ok(CliStreamMessage::Done(result)) => break (*result)?,
            Err(_) => {
                anyhow::bail!("ask request timed out after {} ms", timeout.as_millis());
            }
        }
    };

    match response {
        DaemonResponse::Ask(result) => {
            if !printed_any && !result.output_text.is_empty() {
                writeln!(stdout, "{}", result.output_text)?;
            }
            Ok(())
        }
        DaemonResponse::Error(error) => bail!("{}", error.message),
        other => bail!("unexpected ask response: {other:?}"),
    }
}

fn execute_research_with_terminal_presentation(
    state_root: Option<&Path>,
    args: &ResearchArgs,
) -> Result<()> {
    let config = gptd::AppConfig::load_or_create(state_root)?;
    let paths = build_state_paths(resolve_state_root(state_root));
    let command = build_research_command(args);
    let timeout = Duration::from_millis(config.research_timeout_ms()) + IPC_TIMEOUT_BUFFER;
    let endpoint = paths.ipc_endpoint.clone();
    let (tx, rx) = mpsc::channel();

    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let mut printed_any = false;
    let mut last_status: Option<JobStatusLine> = None;
    thread::spawn(move || {
        let request = DaemonRequest::Research(command);
        let result = gptd::send_request_stream(&endpoint, &request, |event| {
            tx.send(CliStreamMessage::Event(Box::new(event)))
                .map_err(|_| anyhow::anyhow!("CLI stream receiver closed"))?;
            Ok(())
        })
        .with_context(|| format!("connect to daemon at {endpoint}"));
        let _ = tx.send(CliStreamMessage::Done(Box::new(result)));
    });

    let response = loop {
        match rx.recv_timeout(timeout) {
            Ok(CliStreamMessage::Event(event)) => match *event {
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
            },
            Ok(CliStreamMessage::Done(result)) => break (*result)?,
            Err(_) => {
                anyhow::bail!(
                    "research request timed out after {} ms",
                    timeout.as_millis()
                );
            }
        }
    };

    match response {
        DaemonResponse::Research(result) => {
            if !printed_any && !result.report_text.is_empty() {
                writeln!(stdout, "{}", result.report_text)?;
            }
            Ok(())
        }
        DaemonResponse::Error(error) => bail!("{}", error.message),
        other => bail!("unexpected research response: {other:?}"),
    }
}

fn config_show(state_root: Option<&Path>) -> Result<Value> {
    let config = gptd::AppConfig::load_or_create(state_root)?;
    serde_json::to_value(config.snapshot()).context("serialize config snapshot")
}

fn operational_report(state_root: Option<&Path>) -> Result<Value> {
    let config = gptd::AppConfig::load_or_create(state_root)?;
    let connection_mode = config.browser_connection_mode();
    let config_snapshot =
        serde_json::to_value(config.snapshot()).context("serialize config snapshot")?;
    let state_root_path = config.paths.state_root.display().to_string();
    let ipc_endpoint = config.paths.ipc_endpoint.clone();
    let logs_dir = config.paths.logs_dir.display().to_string();
    let daemon_log = config.paths.log_path.display().to_string();
    let gateway_log = config
        .paths
        .logs_dir
        .join("gateway.log")
        .display()
        .to_string();
    let daemon_running = gptd::daemon_is_responsive(&config.paths.ipc_endpoint);
    let daemon_doctor = if daemon_running {
        Some(send_daemon_request(state_root, DaemonRequest::Doctor)?)
    } else {
        None
    };
    let gateway = tablinq_gateway::gateway_doctor(state_root)?;
    let health = daemon_doctor
        .as_ref()
        .and_then(|value| value.get("health"))
        .cloned()
        .or_else(|| {
            gateway
                .get("health")
                .and_then(|value| value.get("health"))
                .cloned()
        })
        .unwrap_or(Value::Null);
    let browser = daemon_doctor
        .as_ref()
        .and_then(|value| value.get("browser"))
        .cloned()
        .unwrap_or(Value::Null);
    let extension_connected = health
        .get("extension_connected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let authenticated = health
        .get("authenticated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let browser_connected = health
        .get("browser_connected")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let queue = gateway.get("scheduler").cloned().unwrap_or(Value::Null);
    let browser_state = browser_runtime_state(
        connection_mode,
        browser.as_object(),
        browser_connected,
        extension_connected,
        authenticated,
    );

    Ok(json!({
        "service": "tablinq",
        "state_root": state_root_path,
        "config": config_snapshot,
        "daemon": {
            "running": daemon_running,
            "ipc_endpoint": ipc_endpoint,
            "doctor": daemon_doctor,
        },
        "gateway": gateway,
        "browser": {
            "state": browser_state,
            "connected": browser_connected,
            "snapshot": browser,
        },
        "extension": {
            "connected": extension_connected,
        },
        "auth": {
            "authenticated": authenticated,
        },
        "queue": queue,
        "logs": {
            "directory": logs_dir,
            "daemon": daemon_log,
            "gateway": gateway_log,
        },
        "safe_parallelism": {
            "max_parallel_requests": 1,
            "max_parallel_branches": 1,
            "max_subagents": 0,
        },
        "recovery_commands": browser_recovery_commands(connection_mode),
    }))
}

fn browser_runtime_state(
    mode: BrowserConnectionMode,
    browser: Option<&serde_json::Map<String, Value>>,
    browser_connected: bool,
    extension_connected: bool,
    authenticated: bool,
) -> &'static str {
    let target_tab_found = browser
        .and_then(|value| value.get("target_tab_found"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let browser_owner = browser
        .and_then(|value| value.get("browser_owner"))
        .and_then(Value::as_str)
        .unwrap_or(if is_attach_primary(mode) {
            "external"
        } else {
            "managed"
        });

    if browser_owner == "external" || (is_attach_primary(mode) && !browser_connected) {
        if !browser_connected {
            "attach_unavailable"
        } else if !target_tab_found {
            "target_tab_missing"
        } else if !authenticated {
            "attached_signed_out"
        } else if !extension_connected {
            "attached_extension_disconnected"
        } else {
            "attached_ready"
        }
    } else if !browser_connected {
        "managed_not_started"
    } else if !authenticated {
        "managed_signed_out"
    } else if !extension_connected {
        "managed_extension_disconnected"
    } else {
        "managed_ready"
    }
}

fn browser_recovery_commands(mode: BrowserConnectionMode) -> Vec<&'static str> {
    let mut commands = vec!["gpt daemon start", "gpt gateway start"];
    if is_attach_primary(mode) {
        commands.push("gpt browser attach");
        commands.push("gpt browser connection-info");
        commands.push("gpt browser open");
    } else {
        commands.push("gpt browser open");
        commands.push("gpt browser reset-profile");
    }
    commands.push("gpt gateway stop");
    commands.push("gpt daemon stop");
    commands
}

fn is_attach_primary(mode: BrowserConnectionMode) -> bool {
    matches!(
        mode,
        BrowserConnectionMode::AttachFirst | BrowserConnectionMode::ManagedFallback
    )
}

fn supports_managed_profile_reset(mode: BrowserConnectionMode) -> bool {
    matches!(
        mode,
        BrowserConnectionMode::ManagedFallback | BrowserConnectionMode::ManagedOnly
    )
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

enum CliStreamMessage {
    Event(Box<EventPayload>),
    Done(Box<Result<DaemonResponse>>),
}

fn run_with_timeout<T, F>(timeout: Duration, timeout_message: String, work: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.recv_timeout(timeout)
        .map_err(|_| anyhow::anyhow!(timeout_message))?
}

fn build_ask_command(args: &AskArgs) -> AskCommand {
    AskCommand {
        prompt: args.prompt.clone(),
        conversation_id: args.conversation.clone(),
        new_conversation: false,
        mode: map_mode(args.mode),
        model_class: args.model_class.map(map_model),
        effort: args.effort.map(map_effort),
        files: args
            .files
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
    }
}

fn build_research_command(args: &ResearchArgs) -> ResearchCommand {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

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
    fn render_execution_pretty_prints_json_values() {
        let output = render_execution(CliExecution::Json(json!({
            "status": "ok"
        })))
        .unwrap();

        assert!(output.contains("\"status\": \"ok\""));
        assert!(output.contains('\n'));
    }

    #[test]
    fn config_show_command_renders_pretty_json() {
        let dir = tempdir().unwrap();
        let cli = Cli::parse_from([
            "gpt",
            "--state-root",
            dir.path().to_str().unwrap(),
            "config",
            "show",
        ]);

        let output = run(&cli).unwrap();

        assert!(output.contains("\"state_root\""));
        assert!(output.contains("\"config_path\""));
        assert!(output.contains("\"browser_connection_mode\": \"attach_first\""));
        assert!(output.contains("\"browser_debug_port\": 9222"));
    }

    #[test]
    fn doctor_command_renders_operational_report_without_running_services() {
        let dir = tempdir().unwrap();
        let cli = Cli::parse_from([
            "gpt",
            "--state-root",
            dir.path().to_str().unwrap(),
            "doctor",
        ]);

        let output = run(&cli).unwrap();

        assert!(output.contains("\"config\""));
        assert!(output.contains("\"daemon\""));
        assert!(output.contains("\"gateway\""));
        assert!(output.contains("\"logs\""));
        assert!(output.contains("\"browser_connection_mode\": \"attach_first\""));
        assert!(output.contains("\"state\": \"attach_unavailable\""));
        assert!(output.contains("\"recovery_commands\""));
        assert!(output.contains("\"gpt browser attach\""));
        assert!(output.contains("\"gpt browser connection-info\""));
        assert!(!output.contains("\"gpt browser reset-profile\""));
    }

    #[test]
    fn status_command_renders_attach_first_recovery_commands_without_running_services() {
        let dir = tempdir().unwrap();
        let cli = Cli::parse_from([
            "gpt",
            "--state-root",
            dir.path().to_str().unwrap(),
            "status",
        ]);

        let output = run(&cli).unwrap();

        assert!(output.contains("\"state\": \"attach_unavailable\""));
        assert!(output.contains("\"gpt browser attach\""));
        assert!(output.contains("\"gpt browser connection-info\""));
    }

    #[test]
    fn browser_connection_info_reports_attach_first_defaults() {
        let dir = tempdir().unwrap();
        let cli = Cli::parse_from([
            "gpt",
            "--state-root",
            dir.path().to_str().unwrap(),
            "browser",
            "connection-info",
        ]);

        let output = run(&cli).unwrap();

        assert!(output.contains("\"attach_mode\": \"attach_first\""));
        assert!(output.contains("\"debug_endpoint\": \"http://127.0.0.1:9222\""));
        assert!(output.contains("\"gpt browser attach\""));
    }

    #[test]
    fn reset_profile_is_rejected_in_attach_first_mode() {
        let dir = tempdir().unwrap();
        let cli = Cli::parse_from([
            "gpt",
            "--state-root",
            dir.path().to_str().unwrap(),
            "browser",
            "reset-profile",
        ]);

        let err = run(&cli).unwrap_err();
        assert!(format!("{err:#}").contains("managed_fallback or managed_only"));
    }

    #[test]
    fn run_with_timeout_returns_completed_work() {
        let value = run_with_timeout(Duration::from_millis(100), "timed out".to_owned(), || {
            Ok::<_, anyhow::Error>(42)
        })
        .unwrap();

        assert_eq!(value, 42);
    }

    #[test]
    fn run_with_timeout_fails_for_stalled_work() {
        let err = run_with_timeout(Duration::from_millis(50), "timed out".to_owned(), || {
            std::thread::sleep(Duration::from_millis(200));
            Ok::<_, anyhow::Error>(())
        })
        .unwrap_err();

        assert!(format!("{err:#}").contains("timed out"));
    }
}
