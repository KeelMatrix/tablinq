use crate::cli::{
    AskArgs, ResearchArgs, build_ask_command, build_research_command, resolve_ask_timeout_ms,
};
use crate::daemon::{install_execution_interrupt_sender, request_job_cancellation};
use anyhow::{Context, Result, bail};
use shared_protocol::{
    DaemonRequest, DaemonResponse, EventPayload, JobStatus, StreamChunkKind, build_state_paths,
    resolve_state_root,
};
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::ansi::ansi_green_enabled_for_stdout;
use super::response::{
    ResponseDisplay, ResponseReconciliation, ResponseStreamState, buffer_response_chunk,
    finish_terminal_stream, reconcile_streamed_response_with_pending,
    reconcile_streamed_response_with_rewrite_safety, write_complete_response,
};

const IPC_TIMEOUT_BUFFER: Duration = Duration::from_secs(30);

#[derive(Debug, PartialEq, Eq)]
pub(super) struct JobStatusLine {
    pub(super) status: String,
    pub(super) message: String,
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
    let response_display = if stream_responses {
        ResponseDisplay::InteractiveTerminal
    } else {
        ResponseDisplay::Raw
    };
    let mut stderr = io::stderr();
    let mut response_stream = ResponseStreamState::default();
    let green_responses = ansi_green_enabled_for_stdout();
    let started_at = Instant::now();
    let mut current_job_id: Option<String> = None;
    let mut last_status: Option<JobStatusLine> = None;
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
            finish_terminal_stream(
                &mut stdout,
                response_stream.printed_any,
                &mut response_stream.response_color_active,
            )?;
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
            finish_terminal_stream(
                &mut stdout,
                response_stream.printed_any,
                &mut response_stream.response_color_active,
            )?;
            anyhow::bail!("ask request timed out after {} ms", timeout.as_millis());
        };
        match rx.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(CliStreamMessage::Event(event)) => match *event {
                EventPayload::StreamChunk(chunk) => {
                    current_job_id = Some(chunk.job_id.clone());
                    if chunk.kind == StreamChunkKind::Delta
                        && !chunk.text.is_empty()
                        && stream_responses
                    {
                        buffer_response_chunk(
                            &mut stdout,
                            &chunk.text,
                            green_responses,
                            response_display,
                            &mut response_stream,
                        )?;
                    }
                }
                EventPayload::JobState(state) => {
                    current_job_id = Some(state.job_id.clone());
                    if let Some(next) = ask_progress_status_line(&state)
                        && last_status.as_ref() != Some(&next)
                    {
                        writeln!(stderr, "{}", next.message)?;
                        stderr.flush()?;
                        last_status = Some(next);
                    }
                }
                _ => {}
            },
            Ok(CliStreamMessage::Done(result)) => break (*result)?,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                finish_terminal_stream(
                    &mut stdout,
                    response_stream.printed_any,
                    &mut response_stream.response_color_active,
                )?;
                anyhow::bail!("ask request stream closed before a daemon response arrived");
            }
        }
    };

    match response {
        DaemonResponse::Ask(result) => {
            reconcile_streamed_response_with_pending(
                &mut stdout,
                &response_stream.printed_text,
                &response_stream.pending_text,
                &result.output_text,
                green_responses,
                response_display,
                &mut response_stream.response_color_active,
            )?;
            Ok(())
        }
        DaemonResponse::Error(error) => {
            finish_terminal_stream(
                &mut stdout,
                response_stream.printed_any,
                &mut response_stream.response_color_active,
            )?;
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
    let response_display = if stream_responses {
        ResponseDisplay::InteractiveTerminal
    } else {
        ResponseDisplay::Raw
    };
    let mut stderr = io::stderr();
    let mut response_stream = ResponseStreamState::default();
    let green_responses = ansi_green_enabled_for_stdout();
    let mut last_status: Option<JobStatusLine> = None;
    let mut research_running_announced = false;
    let mut research_plan_printed = false;
    let mut research_result_label_printed = false;
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
            finish_terminal_stream(
                &mut stdout,
                response_stream.printed_any,
                &mut response_stream.response_color_active,
            )?;
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
            finish_terminal_stream(
                &mut stdout,
                response_stream.printed_any,
                &mut response_stream.response_color_active,
            )?;
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
                            if !research_result_label_printed {
                                write_research_result_label(&mut stdout)?;
                                research_result_label_printed = true;
                            }
                            buffer_response_chunk(
                                &mut stdout,
                                &chunk.text,
                                green_responses,
                                response_display,
                                &mut response_stream,
                            )?;
                        }
                    }
                    _ => {}
                },
                EventPayload::JobState(state) => {
                    current_job_id = Some(state.job_id.clone());
                    if let Some(next) =
                        research_progress_status_line(&state, research_running_announced)
                    {
                        if next.status == "running" {
                            research_running_announced = true;
                        }
                        if last_status.as_ref() != Some(&next) {
                            writeln!(stderr, "[{}] {}", next.status, next.message)?;
                            stderr.flush()?;
                            last_status = Some(next);
                        }
                    }
                    if !research_plan_printed
                        && !research_result_label_printed
                        && let Some(message) = state.message.as_deref()
                        && let Some(plan) = extract_research_plan(message)
                    {
                        writeln!(stdout, "RESEARCH PLAN:\n{plan}\n")?;
                        stdout.flush()?;
                        research_plan_printed = true;
                    }
                }
                _ => {}
            },
            Ok(CliStreamMessage::Done(result)) => break (*result)?,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                finish_terminal_stream(
                    &mut stdout,
                    response_stream.printed_any,
                    &mut response_stream.response_color_active,
                )?;
                anyhow::bail!("research request stream closed before a daemon response arrived");
            }
        }
    };

    match response {
        DaemonResponse::Research(result) => {
            if stream_responses {
                if !result.report_text.is_empty() && !research_result_label_printed {
                    write_research_result_label(&mut stdout)?;
                }
                reconcile_streamed_response_with_rewrite_safety(
                    &mut stdout,
                    ResponseReconciliation {
                        printed_text: &response_stream.printed_text,
                        pending_text: &response_stream.pending_text,
                        final_text: &result.report_text,
                        green_responses,
                        display: response_display,
                        rewrite_in_place_safe: false,
                    },
                    &mut response_stream.response_color_active,
                )?;
            } else if !result.report_text.is_empty() {
                write_research_result_label(&mut stdout)?;
                write_complete_response(
                    &mut stdout,
                    &result.report_text,
                    green_responses,
                    response_display,
                )?;
            }
            Ok(())
        }
        DaemonResponse::Error(error) => {
            finish_terminal_stream(
                &mut stdout,
                response_stream.printed_any,
                &mut response_stream.response_color_active,
            )?;
            bail!("{}", error.message)
        }
        other => bail!("unexpected research response: {other:?}"),
    }
}

pub(super) fn ask_progress_status_line(state: &shared_protocol::JobState) -> Option<JobStatusLine> {
    let message = state.message.as_deref()?.trim();
    if !matches!(
        message,
        "Provider generation failed. Page Retry disabled."
            | "Reloading tab and starting a fresh conversation..."
            | "Replaying request with preserved context snapshot..."
    ) {
        return None;
    }

    Some(JobStatusLine {
        status: format!("{:?}", state.status).to_lowercase(),
        message: message.to_owned(),
    })
}

pub(super) fn research_progress_status_line(
    state: &shared_protocol::JobState,
    running_announced: bool,
) -> Option<JobStatusLine> {
    let status = format!("{:?}", state.status).to_lowercase();
    match state.status {
        JobStatus::Queued => Some(JobStatusLine {
            status,
            message: non_empty_job_message(state)
                .unwrap_or_else(|| "queued research job".to_owned()),
        }),
        JobStatus::WaitingForSources => {
            if non_empty_job_message(state).as_deref() == Some("awaiting start confirmation") {
                Some(JobStatusLine {
                    status,
                    message: "awaiting start confirmation".to_owned(),
                })
            } else {
                None
            }
        }
        JobStatus::Running if !running_announced => Some(JobStatusLine {
            status,
            message: "research started".to_owned(),
        }),
        JobStatus::Failed
        | JobStatus::Cancelled
        | JobStatus::Interrupted
        | JobStatus::Recovered => Some(JobStatusLine {
            status,
            message: non_empty_job_message(state).unwrap_or_default(),
        }),
        _ => None,
    }
}

pub(super) fn extract_research_plan(message: &str) -> Option<String> {
    let lines: Vec<&str> = message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let plan_start = lines.iter().position(|line| {
        matches!(
            line.to_ascii_lowercase().as_str(),
            "update" | "edit" | "cancel"
        )
    })? + 1;

    let mut plan = Vec::new();
    for line in lines.iter().skip(plan_start) {
        let lower = line.to_ascii_lowercase();
        if lower == "stop research"
            || lower == "researching..."
            || lower.ends_with("...")
            || lower.contains("searches")
            || lower.contains("citations")
            || lower == "update"
            || lower == "edit"
            || lower == "cancel"
            || line.chars().all(|ch| ch.is_ascii_digit())
        {
            break;
        }
        plan.push((*line).to_owned());
    }

    if plan.is_empty() {
        None
    } else {
        Some(plan.join("\n"))
    }
}

fn non_empty_job_message(state: &shared_protocol::JobState) -> Option<String> {
    let message = state.message.as_deref()?.trim();
    (!message.is_empty()).then(|| message.to_owned())
}

fn write_research_result_label(stdout: &mut impl Write) -> Result<()> {
    writeln!(stdout, "RESEARCH RESULT:")?;
    stdout.flush()?;
    Ok(())
}
