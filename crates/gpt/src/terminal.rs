use crate::cli::{
    AskArgs, ResearchArgs, build_ask_command, build_research_command, resolve_ask_timeout_ms,
};
use crate::daemon::{install_execution_interrupt_sender, request_job_cancellation};
use anyhow::{Context, Result, bail};
use shared_protocol::{
    DaemonRequest, DaemonResponse, EventPayload, StreamChunkKind, build_state_paths,
    resolve_state_root,
};
use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(test)]
use crate::CliExecution;
#[cfg(test)]
use serde_json::json;

const IPC_TIMEOUT_BUFFER: Duration = Duration::from_secs(30);
const ANSI_RESET: &str = "\x1b[0m";
const ANSI_RED: &str = "\x1b[31m";
pub(crate) const ANSI_GREEN: &str = "\x1b[32m";

#[derive(Debug, PartialEq, Eq)]
struct JobStatusLine {
    status: String,
    message: String,
}

enum CliStreamMessage {
    Event(Box<EventPayload>),
    Done(Box<Result<DaemonResponse>>),
}

pub(crate) fn execute_ask_with_terminal_presentation(
    state_root: Option<&Path>,
    args: &AskArgs,
) -> Result<()> {
    let config = gptd::AppConfig::load_or_create(state_root)?;
    let paths = build_state_paths(resolve_state_root(state_root));
    let command = build_ask_command(args);
    let timeout =
        Duration::from_millis(resolve_ask_timeout_ms(&config, command.effort)) + IPC_TIMEOUT_BUFFER;
    let endpoint = paths.ipc_endpoint.clone();
    let worker_endpoint = endpoint.clone();
    let (tx, rx) = mpsc::channel();
    let (interrupt_tx, interrupt_rx) = mpsc::channel();
    let _interrupt_guard = install_execution_interrupt_sender(interrupt_tx)?;

    let mut stdout = io::stdout();
    let stream_responses = stdout.is_terminal();
    let mut printed_any = false;
    let mut streamed_text = String::new();
    let green_responses = ansi_green_enabled_for_stdout();
    let mut response_color_active = false;
    let started_at = Instant::now();
    let mut current_job_id: Option<String> = None;
    thread::spawn(move || {
        let request = DaemonRequest::Ask(command);
        let result = gptd::send_request_stream(&worker_endpoint, &request, |event| {
            tx.send(CliStreamMessage::Event(Box::new(event)))
                .map_err(|_| anyhow::anyhow!("CLI stream receiver closed"))?;
            Ok(())
        })
        .with_context(|| format!("connect to daemon at {worker_endpoint}"));
        let _ = tx.send(CliStreamMessage::Done(Box::new(result)));
    });

    let response = loop {
        if interrupt_rx.try_recv().is_ok() {
            finish_terminal_stream(&mut stdout, printed_any, &mut response_color_active)?;
            if let Some(job_id) = current_job_id.as_deref() {
                let cancel_notice = request_job_cancellation(&endpoint, job_id).err();
                if let Some(err) = cancel_notice {
                    anyhow::bail!(
                        "ask cancelled by Ctrl+C (failed to notify daemon about {}: {err:#})",
                        job_id
                    );
                }
            }
            anyhow::bail!("ask cancelled by Ctrl+C");
        }

        let Some(remaining) = timeout.checked_sub(started_at.elapsed()) else {
            finish_terminal_stream(&mut stdout, printed_any, &mut response_color_active)?;
            anyhow::bail!("ask request timed out after {} ms", timeout.as_millis());
        };
        match rx.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(CliStreamMessage::Event(event)) => match *event {
                EventPayload::StreamChunk(chunk) => {
                    current_job_id = Some(chunk.job_id.clone());
                    if chunk.kind == StreamChunkKind::Delta && !chunk.text.is_empty() {
                        streamed_text.push_str(&chunk.text);
                        if stream_responses {
                            write_response_chunk(
                                &mut stdout,
                                &chunk.text,
                                green_responses,
                                &mut response_color_active,
                            )?;
                            printed_any = true;
                        }
                    }
                }
                EventPayload::JobState(state) => {
                    current_job_id = Some(state.job_id);
                }
                _ => {}
            },
            Ok(CliStreamMessage::Done(result)) => break (*result)?,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                finish_terminal_stream(&mut stdout, printed_any, &mut response_color_active)?;
                anyhow::bail!("ask request stream closed before a daemon response arrived");
            }
        }
    };

    match response {
        DaemonResponse::Ask(result) => {
            reconcile_streamed_response(
                &mut stdout,
                &streamed_text,
                &result.output_text,
                printed_any,
                green_responses,
                &mut response_color_active,
            )?;
            Ok(())
        }
        DaemonResponse::Error(error) => {
            finish_terminal_stream(&mut stdout, printed_any, &mut response_color_active)?;
            bail!("{}", error.message)
        }
        other => bail!("unexpected ask response: {other:?}"),
    }
}

pub(crate) fn execute_research_with_terminal_presentation(
    state_root: Option<&Path>,
    args: &ResearchArgs,
) -> Result<()> {
    let config = gptd::AppConfig::load_or_create(state_root)?;
    let paths = build_state_paths(resolve_state_root(state_root));
    let command = build_research_command(args);
    let timeout = Duration::from_millis(config.research_timeout_ms()) + IPC_TIMEOUT_BUFFER;
    let endpoint = paths.ipc_endpoint.clone();
    let worker_endpoint = endpoint.clone();
    let (tx, rx) = mpsc::channel();
    let (interrupt_tx, interrupt_rx) = mpsc::channel();
    let _interrupt_guard = install_execution_interrupt_sender(interrupt_tx)?;

    let mut stdout = io::stdout();
    let stream_responses = stdout.is_terminal();
    let mut stderr = io::stderr();
    let mut printed_any = false;
    let green_responses = ansi_green_enabled_for_stdout();
    let mut response_color_active = false;
    let mut last_status: Option<JobStatusLine> = None;
    let started_at = Instant::now();
    let mut current_job_id: Option<String> = None;
    thread::spawn(move || {
        let request = DaemonRequest::Research(command);
        let result = gptd::send_request_stream(&worker_endpoint, &request, |event| {
            tx.send(CliStreamMessage::Event(Box::new(event)))
                .map_err(|_| anyhow::anyhow!("CLI stream receiver closed"))?;
            Ok(())
        })
        .with_context(|| format!("connect to daemon at {worker_endpoint}"));
        let _ = tx.send(CliStreamMessage::Done(Box::new(result)));
    });

    let response = loop {
        if interrupt_rx.try_recv().is_ok() {
            finish_terminal_stream(&mut stdout, printed_any, &mut response_color_active)?;
            if let Some(job_id) = current_job_id.as_deref() {
                let cancel_notice = request_job_cancellation(&endpoint, job_id).err();
                if let Some(err) = cancel_notice {
                    anyhow::bail!(
                        "research cancelled by Ctrl+C (failed to notify daemon about {}: {err:#})",
                        job_id
                    );
                }
            }
            anyhow::bail!("research cancelled by Ctrl+C");
        }

        let Some(remaining) = timeout.checked_sub(started_at.elapsed()) else {
            finish_terminal_stream(&mut stdout, printed_any, &mut response_color_active)?;
            anyhow::bail!(
                "research request timed out after {} ms",
                timeout.as_millis()
            );
        };
        match rx.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(CliStreamMessage::Event(event)) => match *event {
                EventPayload::StreamChunk(chunk) => match chunk.kind {
                    StreamChunkKind::Delta if !chunk.text.is_empty() => {
                        current_job_id = Some(chunk.job_id.clone());
                        if stream_responses {
                            write_response_chunk(
                                &mut stdout,
                                &chunk.text,
                                green_responses,
                                &mut response_color_active,
                            )?;
                            printed_any = true;
                        }
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
                    current_job_id = Some(state.job_id.clone());
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
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                finish_terminal_stream(&mut stdout, printed_any, &mut response_color_active)?;
                anyhow::bail!("research request stream closed before a daemon response arrived");
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

pub(crate) fn reconcile_streamed_response(
    stdout: &mut impl Write,
    streamed_text: &str,
    final_text: &str,
    printed_any: bool,
    green_responses: bool,
    response_color_active: &mut bool,
) -> Result<()> {
    if !printed_any {
        if !final_text.is_empty() {
            write_complete_response(stdout, final_text, green_responses)?;
        }
        return Ok(());
    }

    if streamed_text == final_text {
        return finish_terminal_stream(stdout, true, response_color_active);
    }

    if let Some(suffix) = final_text.strip_prefix(streamed_text) {
        if !suffix.is_empty() {
            write_response_chunk(stdout, suffix, green_responses, response_color_active)?;
        }
        return finish_terminal_stream(stdout, true, response_color_active);
    }

    if can_rewrite_response_line(streamed_text, final_text) {
        return rewrite_response_line(
            stdout,
            streamed_text,
            final_text,
            green_responses,
            response_color_active,
        );
    }

    finish_terminal_stream(stdout, true, response_color_active)?;
    if !final_text.is_empty() {
        write_complete_response(stdout, final_text, green_responses)?;
    }
    Ok(())
}

fn can_rewrite_response_line(streamed_text: &str, final_text: &str) -> bool {
    !streamed_text.chars().any(|ch| matches!(ch, '\n' | '\r'))
        && !final_text.chars().any(|ch| matches!(ch, '\n' | '\r'))
}

fn rewrite_response_line(
    stdout: &mut impl Write,
    streamed_text: &str,
    final_text: &str,
    green_responses: bool,
    response_color_active: &mut bool,
) -> Result<()> {
    if *response_color_active {
        write!(stdout, "{ANSI_RESET}")?;
        *response_color_active = false;
    }

    let clear_width = streamed_text
        .chars()
        .count()
        .saturating_sub(final_text.chars().count());
    let clear_padding = " ".repeat(clear_width);
    if green_responses {
        writeln!(
            stdout,
            "\r{ANSI_GREEN}{final_text}{ANSI_RESET}{clear_padding}"
        )?;
    } else {
        writeln!(stdout, "\r{final_text}{clear_padding}")?;
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
    ansi_color_supported(&env, windows_stdout_supports_ansi()) && shell_background_is_black(&env)
}

fn ansi_color_enabled_for_stderr() -> bool {
    if !io::stderr().is_terminal() {
        return false;
    }
    ansi_color_supported(
        &env::vars().collect::<Vec<_>>(),
        windows_stderr_supports_ansi(),
    )
}

fn ansi_color_supported(env: &[(String, String)], windows_console_support: Option<bool>) -> bool {
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
        return windows_console_support.unwrap_or(false)
            || get("WT_SESSION").is_some()
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
fn windows_stdout_supports_ansi() -> Option<bool> {
    windows_console_supports_ansi(-11)
}

#[cfg(not(windows))]
fn windows_stdout_supports_ansi() -> Option<bool> {
    None
}

#[cfg(windows)]
fn windows_stderr_supports_ansi() -> Option<bool> {
    windows_console_supports_ansi(-12)
}

#[cfg(not(windows))]
fn windows_stderr_supports_ansi() -> Option<bool> {
    None
}

#[cfg(windows)]
fn windows_console_supports_ansi(std_handle: i32) -> Option<bool> {
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

    unsafe extern "system" {
        fn GetStdHandle(n_std_handle: i32) -> *mut std::ffi::c_void;
        fn GetConsoleMode(h_console_handle: *mut std::ffi::c_void, lp_mode: *mut u32) -> i32;
        fn SetConsoleMode(h_console_handle: *mut std::ffi::c_void, dw_mode: u32) -> i32;
    }

    let handle = unsafe { GetStdHandle(std_handle) };
    if handle.is_null() || handle as isize == -1 {
        return None;
    }

    let mut mode = 0u32;
    if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
        return None;
    }

    if (mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0 {
        return Some(true);
    }

    let enabled_mode = mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING;
    Some(unsafe { SetConsoleMode(handle, enabled_mode) != 0 })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_execution_pretty_prints_json_values() {
        let output = crate::render_execution(CliExecution::Json(json!({
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
        assert!(!ansi_color_supported(
            &[("NO_COLOR".to_owned(), "1".to_owned())],
            None
        ));
        assert!(ansi_color_supported(
            &[("FORCE_COLOR".to_owned(), "1".to_owned())],
            None
        ));
    }

    #[test]
    fn ansi_color_supported_by_env_rejects_dumb_term_without_force() {
        assert!(!ansi_color_supported(
            &[("TERM".to_owned(), "dumb".to_owned())],
            None
        ));
    }

    #[test]
    fn ansi_color_supported_prefers_real_windows_console_capability() {
        assert!(ansi_color_supported(&[], Some(true)));
        assert!(!ansi_color_supported(&[], Some(false)));
    }

    #[test]
    fn ansi_color_supported_force_color_overrides_missing_windows_console_capability() {
        assert!(ansi_color_supported(
            &[("FORCE_COLOR".to_owned(), "1".to_owned())],
            Some(false)
        ));
        assert!(!ansi_color_supported(
            &[
                ("FORCE_COLOR".to_owned(), "1".to_owned()),
                ("NO_COLOR".to_owned(), "1".to_owned())
            ],
            Some(true)
        ));
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

    fn render_reconciled_response(
        streamed_chunks: &[&str],
        final_text: &str,
        green_responses: bool,
    ) -> String {
        let mut stdout = Vec::new();
        let mut response_color_active = false;
        let mut streamed_text = String::new();
        let mut printed_any = false;

        for chunk in streamed_chunks {
            streamed_text.push_str(chunk);
            write_response_chunk(
                &mut stdout,
                chunk,
                green_responses,
                &mut response_color_active,
            )
            .unwrap();
            printed_any = true;
        }

        reconcile_streamed_response(
            &mut stdout,
            &streamed_text,
            final_text,
            printed_any,
            green_responses,
            &mut response_color_active,
        )
        .unwrap();

        String::from_utf8(stdout).unwrap()
    }

    #[test]
    fn reconcile_streamed_response_appends_missing_suffix_and_resets_green_output() {
        assert_eq!(
            render_reconciled_response(&["ASK_O"], "ASK_OK", true),
            format!("{ANSI_GREEN}ASK_OK{ANSI_RESET}\n")
        );
    }

    #[test]
    fn reconcile_streamed_response_rewrites_duplicate_single_line_chunks() {
        assert_eq!(
            render_reconciled_response(&["ASK_", "ASK_"], "ASK_OK", false),
            "ASK_ASK_\rASK_OK  \n"
        );
    }

    #[test]
    fn reconcile_streamed_response_rewrites_divergent_single_line_chunks() {
        assert_eq!(
            render_reconciled_response(&["WRONG"], "ASK_OK", false),
            "WRONG\rASK_OK\n"
        );
    }

    #[test]
    fn reconcile_streamed_response_prints_final_output_when_no_chunks_stream() {
        assert_eq!(render_reconciled_response(&[], "ASK_OK", false), "ASK_OK\n");
    }

    #[cfg(windows)]
    #[test]
    fn console_attributes_background_is_black_reads_console_background_bits() {
        assert!(console_attributes_background_is_black(0));
        assert!(!console_attributes_background_is_black(0x0010));
        assert!(!console_attributes_background_is_black(0x00F0));
    }
}
