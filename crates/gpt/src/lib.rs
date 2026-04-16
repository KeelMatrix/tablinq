mod build_info;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use shared_protocol::{
    AskCommand, BrowserConnectionMode, BrowserOpenRequest, CHROME_ATTACH_PROFILE_NOTE,
    CancelJobCommand, DaemonRequest, DaemonResponse, Effort, EventPayload, Mode, ModelClass,
    PullArtifactCommand, ResearchCommand, StreamChunkKind, build_state_paths, resolve_state_root,
};
use std::env;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const IPC_TIMEOUT_BUFFER: Duration = Duration::from_secs(30);
const ANSI_RESET: &str = "\x1b[0m";
const ANSI_RED: &str = "\x1b[31m";
const ANSI_GREEN: &str = "\x1b[32m";

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
        BrowserCommand::StartDebug {
            target_url,
            browser_path,
        } => browser_start_debug(&config, target_url.as_deref(), browser_path.as_deref()),
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
    let startup = attach_startup_story(config);

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
        "startup_story": startup,
        "recommended_commands": browser_recovery_commands(mode),
        "browser": browser,
    }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BrowserStartDebugPlan {
    executable: String,
    launch_args: Vec<String>,
    target_url: String,
    attach_profile_dir: Option<PathBuf>,
}

fn browser_start_debug(
    config: &gptd::AppConfig,
    target_url: Option<&str>,
    browser_path: Option<&Path>,
) -> Result<Value> {
    let plan = build_browser_start_debug_plan(config, target_url, browser_path)?;
    if let Some(attach_profile_dir) = plan.attach_profile_dir.as_ref() {
        fs::create_dir_all(attach_profile_dir)
            .with_context(|| format!("create {}", attach_profile_dir.display()))?;
    }
    let mut child = ProcessCommand::new(&plan.executable);
    child
        .args(&plan.launch_args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = child.spawn().with_context(|| {
        format!(
            "start browser in debug attach mode with executable {}",
            plan.executable
        )
    })?;

    Ok(json!({
        "status": "started",
        "browser_path": plan.executable,
        "target_url": plan.target_url,
        "debug_endpoint": config.browser_debug_endpoint(),
        "debug_host": config.browser_debug_host(),
        "debug_port": config.browser_debug_port(),
        "attach_profile_dir": plan.attach_profile_dir.as_ref().map(|path| path.display().to_string()),
        "pid": child.id(),
        "launch_args": plan.launch_args,
        "browser_user_owned": true,
        "manual_sign_in_required": true,
        "close_existing_browser_windows_first": true,
        "next_commands": [
            "gpt browser attach",
            "gpt status",
            "gpt doctor",
        ],
        "note": browser_start_debug_note(plan.attach_profile_dir.is_some()),
    }))
}

fn build_browser_start_debug_plan(
    config: &gptd::AppConfig,
    target_url: Option<&str>,
    browser_path: Option<&Path>,
) -> Result<BrowserStartDebugPlan> {
    let executable = resolve_browser_executable(config, browser_path)?;
    let target_url = target_url
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(config.browser_target_url())
        .to_owned();
    let attach_profile_dir =
        chrome_attach_profile_dir_for_start_debug(config, browser_path, &executable);
    let mut launch_args = vec![
        format!("--remote-debugging-port={}", config.browser_debug_port()),
        format!("--remote-debugging-address={}", config.browser_debug_host()),
    ];
    if let Some(attach_profile_dir) = attach_profile_dir.as_ref() {
        launch_args.push(format!("--user-data-dir={}", attach_profile_dir.display()));
    }
    launch_args.extend([
        "--no-first-run".to_owned(),
        "--no-default-browser-check".to_owned(),
        target_url.clone(),
    ]);

    Ok(BrowserStartDebugPlan {
        executable,
        launch_args,
        target_url,
        attach_profile_dir,
    })
}

fn browser_start_debug_note(uses_chrome_attach_profile: bool) -> &'static str {
    if uses_chrome_attach_profile {
        "This starts Chrome in attach-first mode with a dedicated persistent real profile under the Tablinq state root so current Chrome will expose the local debug endpoint. It does not create a managed automation profile."
    } else {
        "This starts your normal browser with a local debug endpoint. It does not create a managed automation profile."
    }
}

fn chrome_attach_profile_dir_for_start_debug(
    config: &gptd::AppConfig,
    browser_path: Option<&Path>,
    executable: &str,
) -> Option<PathBuf> {
    if uses_chrome_attach_profile(config, browser_path, executable) {
        Some(config.paths.chrome_attach_profile_dir.clone())
    } else {
        None
    }
}

fn uses_chrome_attach_profile(
    config: &gptd::AppConfig,
    browser_path: Option<&Path>,
    executable: &str,
) -> bool {
    if executable_looks_like_edge(executable) {
        return false;
    }
    if executable_looks_like_chrome(executable) {
        return true;
    }

    let has_explicit_browser_path = browser_path.is_some() || config.browser_path().is_some();
    !has_explicit_browser_path && config.browser_channel().eq_ignore_ascii_case("chrome")
}

fn executable_looks_like_edge(executable: &str) -> bool {
    let lower = executable.to_ascii_lowercase();
    lower.contains("msedge") || lower.contains("microsoft edge")
}

fn executable_looks_like_chrome(executable: &str) -> bool {
    let lower = executable.to_ascii_lowercase();
    lower.contains("google chrome") || lower.contains("chrome.exe") || lower.contains("chrome")
}

fn resolve_browser_executable(
    config: &gptd::AppConfig,
    browser_path: Option<&Path>,
) -> Result<String> {
    if let Some(browser_path) = browser_path {
        return Ok(browser_path.display().to_string());
    }
    if let Some(browser_path) = config.browser_path() {
        return Ok(browser_path.to_owned());
    }
    if let Ok(browser_path) = env::var("TABLINQ_BROWSER_PATH") {
        let browser_path = browser_path.trim();
        if !browser_path.is_empty() {
            return Ok(browser_path.to_owned());
        }
    }

    detect_browser_executable(config.browser_channel()).ok_or_else(|| {
        anyhow::anyhow!(
            "no supported browser executable found for channel {}; set browser_path in config or pass --browser-path",
            config.browser_channel()
        )
    })
}

fn detect_browser_executable(channel: &str) -> Option<String> {
    if cfg!(windows) {
        detect_windows_browser_executable(channel)
    } else if cfg!(target_os = "macos") {
        let mut candidates = ordered_browser_candidates(
            channel,
            &[
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            ],
        );
        candidates.retain(|candidate| Path::new(candidate).exists());
        candidates.into_iter().next().map(str::to_owned)
    } else {
        ordered_browser_candidates(
            channel,
            &[
                "google-chrome",
                "chromium",
                "chromium-browser",
                "microsoft-edge",
            ],
        )
        .into_iter()
        .find_map(resolve_command_in_path)
    }
}

fn ordered_browser_candidates<'a>(channel: &str, candidates: &'a [&'a str]) -> Vec<&'a str> {
    let (preferred, fallback) = if channel.eq_ignore_ascii_case("msedge") {
        (
            vec!["microsoft-edge", "msedge.exe"],
            vec!["google chrome", "chrome.exe"],
        )
    } else {
        (
            vec!["google chrome", "chrome.exe"],
            vec!["microsoft-edge", "msedge.exe"],
        )
    };

    let mut ordered = Vec::new();
    for candidate in candidates {
        let lower = candidate.to_ascii_lowercase();
        if preferred.iter().any(|value| lower.contains(value)) {
            ordered.push(*candidate);
        }
    }
    for candidate in candidates {
        let lower = candidate.to_ascii_lowercase();
        if fallback.iter().any(|value| lower.contains(value)) && !ordered.contains(candidate) {
            ordered.push(*candidate);
        }
    }
    for candidate in candidates {
        if !ordered.contains(candidate) {
            ordered.push(*candidate);
        }
    }
    ordered
}

fn resolve_command_in_path(command: &str) -> Option<String> {
    let tool = if cfg!(windows) { "where" } else { "which" };
    ProcessCommand::new(tool)
        .arg(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| {
            String::from_utf8(output.stdout).ok().and_then(|value| {
                value
                    .lines()
                    .map(str::trim)
                    .find(|value| !value.is_empty())
                    .map(str::to_owned)
            })
        })
}

fn detect_windows_browser_executable(channel: &str) -> Option<String> {
    let local_app_data = env::var("LOCALAPPDATA")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let program_files = [
        env::var("ProgramFiles(x86)").ok(),
        env::var("ProgramFiles").ok(),
    ]
    .into_iter()
    .flatten()
    .filter(|value| !value.trim().is_empty())
    .collect::<Vec<_>>();

    let chrome_candidates = std::iter::once(local_app_data.as_deref().map(|base| {
        Path::new(base)
            .join("Google")
            .join("Chrome")
            .join("Application")
            .join("chrome.exe")
    }))
    .flatten()
    .chain(program_files.iter().map(|base| {
        Path::new(base)
            .join("Google")
            .join("Chrome")
            .join("Application")
            .join("chrome.exe")
    }))
    .collect::<Vec<_>>();
    let edge_candidates = std::iter::once(local_app_data.as_deref().map(|base| {
        Path::new(base)
            .join("Microsoft")
            .join("Edge")
            .join("Application")
            .join("msedge.exe")
    }))
    .flatten()
    .chain(program_files.iter().map(|base| {
        Path::new(base)
            .join("Microsoft")
            .join("Edge")
            .join("Application")
            .join("msedge.exe")
    }))
    .collect::<Vec<_>>();

    let ordered_paths = if channel.eq_ignore_ascii_case("msedge") {
        edge_candidates
            .into_iter()
            .chain(chrome_candidates)
            .collect::<Vec<_>>()
    } else {
        chrome_candidates
            .into_iter()
            .chain(edge_candidates)
            .collect::<Vec<_>>()
    };
    for candidate in ordered_paths {
        if candidate.exists() {
            return Some(candidate.display().to_string());
        }
    }

    let ordered_commands = if channel.eq_ignore_ascii_case("msedge") {
        vec!["msedge.exe", "chrome.exe"]
    } else {
        vec!["chrome.exe", "msedge.exe"]
    };
    ordered_commands
        .into_iter()
        .find_map(resolve_command_in_path)
}

fn attach_startup_story(config: &gptd::AppConfig) -> Value {
    if !is_attach_primary(config.browser_connection_mode()) {
        return json!({
            "mode": "managed_only",
            "command": "gpt browser open",
        });
    }

    let preview = match build_browser_start_debug_plan(config, None, None) {
        Ok(plan) => json!({
            "browser_path": plan.executable,
            "launch_args": plan.launch_args,
            "attach_profile_dir": plan.attach_profile_dir.as_ref().map(|path| path.display().to_string()),
            "profile_note": plan.attach_profile_dir.as_ref().map(|_| CHROME_ATTACH_PROFILE_NOTE),
        }),
        Err(error) => json!({
            "browser_path": Value::Null,
            "launch_error": error.to_string(),
        }),
    };

    json!({
        "mode": "start_debug",
        "command": "gpt browser start-debug",
        "browser_user_owned": true,
        "manual_sign_in_required": true,
        "close_existing_browser_windows_first": true,
        "extension_install_required": config.browser_extension_required(),
        "verify_commands": [
            "gpt browser attach",
            "gpt status",
            "gpt doctor",
            "gpt browser connection-info",
        ],
        "fallback": {
            "mode": "managed_only",
            "config_toml": "browser_connection_mode = \"managed_only\"",
            "command": "gpt browser open",
            "when_to_use": "Use managed_only only for debugging, CI/local harness isolation, or when your normal browser cannot expose the local debug endpoint."
        },
        "preview": preview,
    })
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
    let green_responses = ansi_green_enabled_for_stdout();
    let mut response_color_active = false;
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
                        write_response_chunk(
                            &mut stdout,
                            &chunk.text,
                            green_responses,
                            &mut response_color_active,
                        )?;
                        printed_any = true;
                    }
                    if chunk.kind == StreamChunkKind::Done && printed_any {
                        finish_terminal_stream(
                            &mut stdout,
                            printed_any,
                            &mut response_color_active,
                        )?;
                    }
                }
            }
            Ok(CliStreamMessage::Done(result)) => break (*result)?,
            Err(_) => {
                finish_terminal_stream(&mut stdout, printed_any, &mut response_color_active)?;
                anyhow::bail!("ask request timed out after {} ms", timeout.as_millis());
            }
        }
    };

    match response {
        DaemonResponse::Ask(result) => {
            if !printed_any && !result.output_text.is_empty() {
                write_complete_response(&mut stdout, &result.output_text, green_responses)?;
            }
            Ok(())
        }
        DaemonResponse::Error(error) => {
            finish_terminal_stream(&mut stdout, printed_any, &mut response_color_active)?;
            bail!("{}", error.message)
        }
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
    let green_responses = ansi_green_enabled_for_stdout();
    let mut response_color_active = false;
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
                        write_response_chunk(
                            &mut stdout,
                            &chunk.text,
                            green_responses,
                            &mut response_color_active,
                        )?;
                        printed_any = true;
                    }
                    StreamChunkKind::Done if printed_any => {
                        finish_terminal_stream(
                            &mut stdout,
                            printed_any,
                            &mut response_color_active,
                        )?;
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
                finish_terminal_stream(&mut stdout, printed_any, &mut response_color_active)?;
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
                write_complete_response(&mut stdout, &result.report_text, green_responses)?;
            }
            Ok(())
        }
        DaemonResponse::Error(error) => {
            finish_terminal_stream(&mut stdout, printed_any, &mut response_color_active)?;
            bail!("{}", error.message)
        }
        other => bail!("unexpected research response: {other:?}"),
    }
}

fn write_response_chunk(
    stdout: &mut impl Write,
    text: &str,
    green_responses: bool,
    response_color_active: &mut bool,
) -> Result<()> {
    if green_responses && !*response_color_active {
        write!(stdout, "{ANSI_GREEN}")?;
        *response_color_active = true;
    }
    write!(stdout, "{text}")?;
    stdout.flush()?;
    Ok(())
}

fn write_complete_response(
    stdout: &mut impl Write,
    text: &str,
    green_responses: bool,
) -> Result<()> {
    if green_responses {
        writeln!(stdout, "{ANSI_GREEN}{text}{ANSI_RESET}")?;
    } else {
        writeln!(stdout, "{text}")?;
    }
    stdout.flush()?;
    Ok(())
}

fn finish_terminal_stream(
    stdout: &mut impl Write,
    printed_any: bool,
    response_color_active: &mut bool,
) -> Result<()> {
    if *response_color_active {
        write!(stdout, "{ANSI_RESET}")?;
        *response_color_active = false;
    }
    if printed_any {
        writeln!(stdout)?;
        stdout.flush()?;
    }
    Ok(())
}

pub fn format_cli_error(err: &anyhow::Error) -> String {
    format_cli_error_with_color(err, ansi_color_enabled_for_stderr())
}

fn format_cli_error_with_color(err: &anyhow::Error, ansi_color: bool) -> String {
    if ansi_color {
        format!("{ANSI_RED}error:{ANSI_RESET} {err:#}")
    } else {
        format!("error: {err:#}")
    }
}

fn ansi_green_enabled_for_stdout() -> bool {
    if !io::stdout().is_terminal() {
        return false;
    }
    let env = env::vars().collect::<Vec<_>>();
    ansi_color_supported_by_env(&env) && shell_background_is_black(&env)
}

fn ansi_color_enabled_for_stderr() -> bool {
    if !io::stderr().is_terminal() {
        return false;
    }
    ansi_color_supported_by_env(&env::vars().collect::<Vec<_>>())
}

fn ansi_color_supported_by_env(env: &[(String, String)]) -> bool {
    let get = |key: &str| {
        env.iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, value)| value.as_str())
    };

    if get("NO_COLOR").is_some() {
        return false;
    }

    if matches!(get("CLICOLOR_FORCE"), Some(value) if value != "0")
        || matches!(get("FORCE_COLOR"), Some(value) if value != "0")
    {
        return true;
    }

    let term = get("TERM").unwrap_or_default();
    if term.eq_ignore_ascii_case("dumb") {
        return false;
    }

    #[cfg(windows)]
    {
        return get("WT_SESSION").is_some()
            || get("ANSICON").is_some()
            || matches!(get("ConEmuANSI"), Some(value) if value.eq_ignore_ascii_case("ON"))
            || (!term.is_empty() && !term.eq_ignore_ascii_case("dumb"));
    }

    #[cfg(not(windows))]
    {
        !term.is_empty()
    }
}

fn shell_background_is_black(env: &[(String, String)]) -> bool {
    #[cfg(windows)]
    if let Some(is_black) = windows_console_background_is_black() {
        return is_black;
    }

    let colorfgbg = env
        .iter()
        .find(|(candidate, _)| candidate == "COLORFGBG")
        .map(|(_, value)| value.as_str());

    let Some(colorfgbg) = colorfgbg else {
        return false;
    };

    colorfgbg
        .rsplit(';')
        .next()
        .and_then(|value| value.trim().parse::<u8>().ok())
        == Some(0)
}

#[cfg(windows)]
fn windows_console_background_is_black() -> Option<bool> {
    const STD_OUTPUT_HANDLE: i32 = -11;

    #[repr(C)]
    struct Coord {
        x: i16,
        y: i16,
    }

    #[repr(C)]
    struct SmallRect {
        left: i16,
        top: i16,
        right: i16,
        bottom: i16,
    }

    #[repr(C)]
    struct ConsoleScreenBufferInfo {
        dw_size: Coord,
        dw_cursor_position: Coord,
        w_attributes: u16,
        sr_window: SmallRect,
        dw_maximum_window_size: Coord,
    }

    unsafe extern "system" {
        fn GetStdHandle(n_std_handle: i32) -> *mut std::ffi::c_void;
        fn GetConsoleScreenBufferInfo(
            h_console_output: *mut std::ffi::c_void,
            lp_console_screen_buffer_info: *mut ConsoleScreenBufferInfo,
        ) -> i32;
    }

    let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if handle.is_null() || handle as isize == -1 {
        return None;
    }

    let mut info = ConsoleScreenBufferInfo {
        dw_size: Coord { x: 0, y: 0 },
        dw_cursor_position: Coord { x: 0, y: 0 },
        w_attributes: 0,
        sr_window: SmallRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        dw_maximum_window_size: Coord { x: 0, y: 0 },
    };
    let ok = unsafe { GetConsoleScreenBufferInfo(handle, &mut info) };
    if ok == 0 {
        return None;
    }

    Some(console_attributes_background_is_black(info.w_attributes))
}

#[cfg(windows)]
fn console_attributes_background_is_black(attributes: u16) -> bool {
    const BACKGROUND_MASK: u16 = 0x00F0;
    (attributes & BACKGROUND_MASK) == 0
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
    let daemon_doctor_payload = daemon_doctor
        .as_ref()
        .and_then(|value| daemon_response_payload(value, "doctor"));
    let gateway = tablinq_gateway::gateway_doctor(state_root)?;
    let health = daemon_doctor_payload
        .and_then(|value| value.get("health"))
        .cloned()
        .or_else(|| {
            gateway
                .get("health")
                .and_then(|value| value.get("health"))
                .cloned()
        })
        .unwrap_or(Value::Null);
    let browser = daemon_doctor_payload
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
    let recovery_guidance =
        operational_recovery_guidance(&config, browser_state, daemon_doctor_payload);

    Ok(json!({
        "service": "tablinq",
        "state_root": state_root_path,
        "config": config_snapshot,
        "daemon": {
            "running": daemon_running,
            "ipc_endpoint": ipc_endpoint,
            "doctor": daemon_doctor_payload.cloned(),
        },
        "gateway": gateway,
        "browser": {
            "state": browser_state,
            "connected": browser_connected,
            "snapshot": browser,
            "startup_story": attach_startup_story(&config),
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
        "recovery_guidance": recovery_guidance,
        "recovery_commands": browser_recovery_commands(connection_mode),
    }))
}

fn daemon_response_payload<'a>(value: &'a Value, response_type: &str) -> Option<&'a Value> {
    if value.get("type").and_then(Value::as_str) == Some(response_type) {
        value.get("payload")
    } else {
        None
    }
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
        commands.push("gpt browser start-debug");
        commands.push("gpt browser connection-info");
        commands.push("gpt browser attach");
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

fn operational_recovery_guidance(
    config: &gptd::AppConfig,
    browser_state: &str,
    daemon_doctor: Option<&Value>,
) -> Value {
    if let Some(guidance) = daemon_doctor.and_then(|value| value.get("recovery_guidance")) {
        return guidance.clone();
    }

    if is_attach_primary(config.browser_connection_mode()) {
        let guidance = match browser_state {
            "attach_unavailable" => vec![
                format!(
                "check the attach endpoint at {}; fully close Chrome or Edge, run `gpt browser start-debug`, then inspect `gpt browser connection-info`",
                    config.browser_debug_endpoint()
                ),
                "confirm the Tablinq extension is installed and enabled in the same normal browser profile".to_owned(),
                "sign in to ChatGPT in that browser and clear any login or challenge page".to_owned(),
                "run `gpt browser attach` and then `gpt browser open` to reattach or focus the configured target tab".to_owned(),
                "only if the local debug endpoint cannot be enabled on this machine, or you need debugging or CI/local harness isolation, set `browser_connection_mode = \"managed_only\"` in config.toml and use `gpt browser open` as fallback".to_owned(),
            ],
            "attached_extension_disconnected" => vec![
                "confirm the Tablinq extension is installed, enabled, and connected in the attached browser profile".to_owned(),
                "if multiple ChatGPT tabs are open, keep only the signed-in tab with the working Tablinq extension, then rerun `gpt browser attach`".to_owned(),
                "only if attach mode cannot be stabilized on this machine, or you need debugging or CI/local harness isolation, set `browser_connection_mode = \"managed_only\"` in config.toml and use `gpt browser open` as fallback".to_owned(),
            ],
            "attached_signed_out" => vec![
                "sign in to ChatGPT in the attached normal browser tab and clear any login or challenge page".to_owned(),
                "rerun `gpt browser attach` and `gpt browser open` after the signed-in tab is visible".to_owned(),
                "if multiple ChatGPT tabs are open, keep only the signed-in tab you want Tablinq to use".to_owned(),
            ],
            "target_tab_missing" => vec![
                "open a signed-in ChatGPT tab in your normal browser or run `gpt browser open` to create or focus one".to_owned(),
                "rerun `gpt browser attach` after the target tab is visible".to_owned(),
                "if multiple ChatGPT tabs are open, close extras before retrying attach".to_owned(),
            ],
            _ => vec![
                "attach-first browser session is ready".to_owned(),
            ],
        };
        return json!(guidance);
    }

    json!([
        "managed mode is fallback/debug-only; use `gpt browser open` to start the managed fallback browser tab",
        "sign in to ChatGPT in the managed fallback tab before running ask or research",
        "use managed mode for debugging, CI/local harness isolation, or when attach-first cannot expose a local debug endpoint",
        "if the managed browser profile is wedged, run `gpt browser reset-profile`",
    ])
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
    fn render_execution_pretty_prints_json_values() {
        let output = render_execution(CliExecution::Json(json!({
            "status": "ok"
        })))
        .unwrap();

        assert!(output.contains("\"status\": \"ok\""));
        assert!(output.contains('\n'));
    }

    #[test]
    fn format_cli_error_prefixes_terminal_failures() {
        let err = anyhow::anyhow!("interrupted ask could not be reconciled");
        assert_eq!(
            format_cli_error_with_color(&err, false),
            "error: interrupted ask could not be reconciled"
        );
    }

    #[test]
    fn format_cli_error_adds_red_prefix_when_color_is_enabled() {
        let err = anyhow::anyhow!("adapter failed");
        assert_eq!(
            format_cli_error_with_color(&err, true),
            "\u{1b}[31merror:\u{1b}[0m adapter failed"
        );
    }

    #[test]
    fn ansi_color_supported_by_env_respects_no_color_and_force_color() {
        assert!(!ansi_color_supported_by_env(&[(
            "NO_COLOR".to_owned(),
            "1".to_owned()
        )]));
        assert!(ansi_color_supported_by_env(&[(
            "FORCE_COLOR".to_owned(),
            "1".to_owned()
        )]));
    }

    #[test]
    fn ansi_color_supported_by_env_rejects_dumb_term_without_force() {
        assert!(!ansi_color_supported_by_env(&[(
            "TERM".to_owned(),
            "dumb".to_owned()
        )]));
    }

    #[test]
    fn shell_background_is_black_requires_explicit_black_background() {
        assert!(shell_background_is_black(&[(
            "COLORFGBG".to_owned(),
            "15;0".to_owned()
        )]));
        assert!(!shell_background_is_black(&[(
            "COLORFGBG".to_owned(),
            "0;15".to_owned()
        )]));
        assert!(!shell_background_is_black(&[]));
    }

    #[cfg(windows)]
    #[test]
    fn console_attributes_background_is_black_reads_console_background_bits() {
        assert!(console_attributes_background_is_black(0));
        assert!(!console_attributes_background_is_black(0x0010));
        assert!(!console_attributes_background_is_black(0x00F0));
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
        assert!(output.contains("\"chrome_attach_profile_dir\""));
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
        assert!(output.contains("\"recovery_guidance\""));
        assert!(output.contains("check the attach endpoint"));
        assert!(output.contains("confirm the Tablinq extension is installed"));
        assert!(output.contains("sign in to ChatGPT in that browser"));
        assert!(output.contains("\"recovery_commands\""));
        assert!(output.contains("\"gpt browser start-debug\""));
        assert!(output.contains("\"gpt browser connection-info\""));
        assert!(output.contains("\"gpt browser attach\""));
        assert!(output.contains("\"browser_connection_mode = \\\"managed_only\\\"\""));
        assert!(!output.contains("\"gpt browser reset-profile\""));
        let parsed: Value = serde_json::from_str(&output).unwrap();
        let commands = parsed
            .get("recovery_commands")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(
            commands[3],
            Value::String("gpt browser connection-info".to_owned())
        );
        assert_eq!(commands[4], Value::String("gpt browser attach".to_owned()));
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
        assert!(output.contains("\"recovery_guidance\""));
        assert!(output.contains("\"gpt browser start-debug\""));
        assert!(output.contains("\"gpt browser connection-info\""));
        assert!(output.contains("\"gpt browser attach\""));
        let parsed: Value = serde_json::from_str(&output).unwrap();
        let commands = parsed
            .get("recovery_commands")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(
            commands[3],
            Value::String("gpt browser connection-info".to_owned())
        );
        assert_eq!(commands[4], Value::String("gpt browser attach".to_owned()));
    }

    #[test]
    fn browser_connection_info_reports_attach_first_defaults() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("config")).unwrap();
        fs::write(
            dir.path().join("config").join("config.toml"),
            "browser_channel = \"chrome\"\n",
        )
        .unwrap();
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
        assert!(output.contains("\"gpt browser start-debug\""));
        assert!(output.contains("\"gpt browser attach\""));
        assert!(output.contains("\"browser_connection_mode = \\\"managed_only\\\"\""));
        assert!(output.contains("\"attach_profile_dir\""));
        assert!(output.contains(CHROME_ATTACH_PROFILE_NOTE));
    }

    #[test]
    fn daemon_response_payload_unwraps_attached_doctor_health() {
        let response = json!({
            "type": "doctor",
            "payload": {
                "health": {
                    "browser_connected": true,
                    "extension_connected": true,
                    "authenticated": true,
                },
                "browser": {
                    "browser_owner": "external",
                    "target_tab_found": true,
                }
            }
        });

        let payload = daemon_response_payload(&response, "doctor").unwrap();
        let health = payload.get("health").unwrap();
        let browser = payload.get("browser").and_then(Value::as_object).unwrap();
        let browser_state = browser_runtime_state(
            BrowserConnectionMode::AttachFirst,
            Some(browser),
            health["browser_connected"].as_bool().unwrap(),
            health["extension_connected"].as_bool().unwrap(),
            health["authenticated"].as_bool().unwrap(),
        );

        assert_eq!(browser_state, "attached_ready");
    }

    #[test]
    fn browser_start_debug_serialized_output_includes_chrome_attach_profile_fields() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("config")).unwrap();
        fs::write(
            dir.path().join("config").join("config.toml"),
            "browser_channel = \"chrome\"\n",
        )
        .unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();
        let browser_path = create_fake_browser_launcher(dir.path(), "chrome");

        let value = browser_start_debug(&config, Some("https://chatgpt.com/"), Some(&browser_path))
            .unwrap();

        let attach_profile_dir = value
            .get("attach_profile_dir")
            .and_then(Value::as_str)
            .unwrap();
        assert_eq!(
            attach_profile_dir,
            config.paths.chrome_attach_profile_dir.display().to_string()
        );
        assert_eq!(
            value.get("note").and_then(Value::as_str),
            Some(browser_start_debug_note(true))
        );
        assert!(
            value["launch_args"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item.as_str() == Some(&format!("--user-data-dir={attach_profile_dir}")))
        );
        assert!(config.paths.chrome_attach_profile_dir.exists());
    }

    #[test]
    fn start_debug_plan_uses_dedicated_chrome_attach_profile() {
        let dir = tempdir().unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();

        let plan = build_browser_start_debug_plan(
            &config,
            Some("https://chatgpt.com/"),
            Some(Path::new(
                "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
            )),
        )
        .unwrap();

        assert_eq!(
            plan.executable,
            "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe"
        );
        assert!(
            plan.launch_args
                .contains(&"--remote-debugging-port=9222".to_owned())
        );
        assert!(
            plan.launch_args
                .contains(&"--remote-debugging-address=127.0.0.1".to_owned())
        );
        assert_eq!(
            plan.attach_profile_dir,
            Some(config.paths.chrome_attach_profile_dir.clone())
        );
        assert_eq!(
            config.paths.chrome_attach_profile_dir,
            dir.path().join("chrome-attach-profile")
        );
        assert!(
            config
                .paths
                .chrome_attach_profile_dir
                .starts_with(dir.path())
        );
        assert!(plan.launch_args.contains(&format!(
            "--user-data-dir={}",
            config.paths.chrome_attach_profile_dir.display()
        )));
        assert_ne!(
            config.paths.chrome_attach_profile_dir,
            config.paths.browser_profile_dir
        );
    }

    #[test]
    fn start_debug_plan_leaves_edge_without_attach_profile_flag() {
        let dir = tempdir().unwrap();
        let config = gptd::AppConfig::load_or_create(Some(dir.path())).unwrap();

        let plan = build_browser_start_debug_plan(
            &config,
            Some("https://chatgpt.com/"),
            Some(Path::new("C:\\Browsers\\Edge\\msedge.exe")),
        )
        .unwrap();

        assert_eq!(plan.executable, "C:\\Browsers\\Edge\\msedge.exe");
        assert_eq!(plan.attach_profile_dir, None);
        assert!(
            !plan
                .launch_args
                .iter()
                .any(|arg| arg.contains("--user-data-dir"))
        );
    }

    fn create_fake_browser_launcher(root: &Path, name: &str) -> PathBuf {
        #[cfg(windows)]
        let path = root.join(format!("{name}.cmd"));
        #[cfg(not(windows))]
        let path = root.join(name);

        #[cfg(windows)]
        fs::write(&path, "@echo off\r\nexit /b 0\r\n").unwrap();

        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;

            fs::write(&path, "#!/bin/sh\nexit 0\n").unwrap();
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).unwrap();
        }

        path
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
