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

#[derive(Debug, Default)]
struct ResponseStreamState {
    printed_text: String,
    pending_text: String,
    printed_any: bool,
    response_color_active: bool,
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

fn buffer_response_chunk(
    stdout: &mut impl Write,
    text: &str,
    green_responses: bool,
    response_stream: &mut ResponseStreamState,
) -> Result<()> {
    response_stream.pending_text.push_str(text);
    let (stable_text, unstable_suffix) = split_stable_prefix(&response_stream.pending_text);
    if stable_text.is_empty() {
        return Ok(());
    }

    let stable_text = stable_text.to_owned();
    let unstable_suffix = unstable_suffix.to_owned();
    write_response_chunk(
        stdout,
        &stable_text,
        green_responses,
        &mut response_stream.response_color_active,
    )?;
    response_stream.printed_text.push_str(&stable_text);
    response_stream.pending_text = unstable_suffix;
    response_stream.printed_any = true;
    Ok(())
}

fn split_stable_prefix(text: &str) -> (&str, &str) {
    let unstable_start = find_unstable_suffix_start(text).unwrap_or(text.len());
    text.split_at(unstable_start)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DelimiterRun {
    start: usize,
    len: usize,
    ch: char,
}

fn find_unstable_suffix_start(text: &str) -> Option<usize> {
    [
        find_unclosed_backtick_start(text),
        find_unclosed_emphasis_start(text),
        find_unclosed_link_or_image_start(text),
        find_unclosed_angle_construct_start(text),
        find_trailing_construct_sequence_start(text),
        find_trailing_block_construct_start(text),
        find_trailing_delimiter_run_start(text),
    ]
    .into_iter()
    .flatten()
    .min()
}

fn find_unclosed_backtick_start(text: &str) -> Option<usize> {
    let mut open_runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;

    while i < text.len() {
        let ch = char_at(text, i);
        if ch == '`' && !is_escaped_markdown_delimiter(text, i) {
            let run_len = delimiter_run_len(text, i, '`');
            if let Some(position) = open_runs.iter().rposition(|(_, len)| *len == run_len) {
                open_runs.remove(position);
            } else {
                open_runs.push((i, run_len));
            }
            i += run_len;
            continue;
        }

        i += ch.len_utf8();
    }

    open_runs.into_iter().map(|(start, _)| start).min()
}

fn find_unclosed_emphasis_start(text: &str) -> Option<usize> {
    let mut open_code_runs: Vec<usize> = Vec::new();
    let mut open_delimiters: Vec<DelimiterRun> = Vec::new();
    let mut i = 0;

    while i < text.len() {
        let ch = char_at(text, i);

        if ch == '`' && !is_escaped_markdown_delimiter(text, i) {
            let run_len = delimiter_run_len(text, i, '`');
            if let Some(position) = open_code_runs.iter().rposition(|len| *len == run_len) {
                open_code_runs.remove(position);
            } else {
                open_code_runs.push(run_len);
            }
            i += run_len;
            continue;
        }

        if !open_code_runs.is_empty() {
            i += ch.len_utf8();
            continue;
        }

        if matches!(ch, '*' | '_' | '~') && !is_escaped_markdown_delimiter(text, i) {
            let raw_run_len = delimiter_run_len(text, i, ch);
            let run_len = match ch {
                '~' if raw_run_len >= 2 => 2,
                '~' => 0,
                _ => raw_run_len.min(3),
            };

            if run_len > 0 {
                if can_close_emphasis(text, i, run_len) {
                    if let Some(position) = open_delimiters
                        .iter()
                        .rposition(|run| run.ch == ch && run.len == run_len)
                    {
                        open_delimiters.remove(position);
                        i += run_len;
                        continue;
                    }
                }

                if can_open_emphasis(text, i, run_len) {
                    open_delimiters.push(DelimiterRun {
                        start: i,
                        len: run_len,
                        ch,
                    });
                    i += run_len;
                    continue;
                }
            }
        }

        i += ch.len_utf8();
    }

    open_delimiters.into_iter().map(|run| run.start).min()
}

fn find_unclosed_link_or_image_start(text: &str) -> Option<usize> {
    let mut open_code_runs: Vec<usize> = Vec::new();
    let mut open_brackets: Vec<usize> = Vec::new();
    let mut i = 0;

    while i < text.len() {
        let ch = char_at(text, i);

        if ch == '`' && !is_escaped_markdown_delimiter(text, i) {
            let run_len = delimiter_run_len(text, i, '`');
            if let Some(position) = open_code_runs.iter().rposition(|len| *len == run_len) {
                open_code_runs.remove(position);
            } else {
                open_code_runs.push(run_len);
            }
            i += run_len;
            continue;
        }

        if !open_code_runs.is_empty() {
            i += ch.len_utf8();
            continue;
        }

        if text[i..].starts_with("![") && !is_escaped_markdown_delimiter(text, i) {
            open_brackets.push(i);
            i += 2;
            continue;
        }

        if ch == '[' && !is_escaped_markdown_delimiter(text, i) {
            if i == 0 || text[..i].chars().next_back() != Some('!') {
                open_brackets.push(i);
            }
            i += ch.len_utf8();
            continue;
        }

        if ch == ']' && !is_escaped_markdown_delimiter(text, i) {
            open_brackets.pop();
            i += ch.len_utf8();
            continue;
        }

        i += ch.len_utf8();
    }

    [
        open_brackets.into_iter().min(),
        find_unclosed_link_destination_start(text),
    ]
    .into_iter()
    .flatten()
    .min()
}

fn find_unclosed_link_destination_start(text: &str) -> Option<usize> {
    let mut candidate = None;
    let mut i = 0;

    while i < text.len() {
        let ch = char_at(text, i);
        if ch == ']' && !is_escaped_markdown_delimiter(text, i) {
            let open_paren = i + ch.len_utf8();
            if open_paren < text.len() && text[open_paren..].starts_with('(') {
                if let Some(open_bracket) = find_matching_open_bracket(text, i) {
                    if find_matching_close_paren(text, open_paren).is_none() {
                        candidate = Some(include_image_bang(text, open_bracket));
                    }
                }
            }
        }

        i += ch.len_utf8();
    }

    candidate
}

fn find_unclosed_angle_construct_start(text: &str) -> Option<usize> {
    let mut open_code_runs: Vec<usize> = Vec::new();
    let mut open_angles: Vec<usize> = Vec::new();
    let mut i = 0;

    while i < text.len() {
        let ch = char_at(text, i);

        if ch == '`' && !is_escaped_markdown_delimiter(text, i) {
            let run_len = delimiter_run_len(text, i, '`');
            if let Some(position) = open_code_runs.iter().rposition(|len| *len == run_len) {
                open_code_runs.remove(position);
            } else {
                open_code_runs.push(run_len);
            }
            i += run_len;
            continue;
        }

        if !open_code_runs.is_empty() {
            i += ch.len_utf8();
            continue;
        }

        if ch == '<'
            && !is_escaped_markdown_delimiter(text, i)
            && looks_like_angle_construct_start(text, i)
        {
            open_angles.push(i);
            i += ch.len_utf8();
            continue;
        }

        if ch == '>' && !is_escaped_markdown_delimiter(text, i) {
            open_angles.pop();
        }

        i += ch.len_utf8();
    }

    open_angles.into_iter().min()
}

fn find_trailing_construct_sequence_start(text: &str) -> Option<usize> {
    let mut end = text.len();
    let mut last_start = None;

    loop {
        let Some(start) = match_trailing_complete_construct_start(text, end) else {
            break;
        };
        last_start = Some(start);
        end = start;

        let trimmed_end = trim_trailing_whitespace(text, end);
        if trimmed_end == end
            || match_trailing_complete_construct_start(text, trimmed_end).is_none()
        {
            break;
        }

        end = trimmed_end;
    }

    last_start
}

fn match_trailing_complete_construct_start(text: &str, end: usize) -> Option<usize> {
    match_trailing_link_or_image_start(text, end)
        .or_else(|| match_trailing_html_or_angle_start(text, end))
        .or_else(|| match_trailing_backtick_construct_start(text, end))
        .or_else(|| match_trailing_emphasis_start(text, end))
}

fn match_trailing_link_or_image_start(text: &str, end: usize) -> Option<usize> {
    let trimmed = &text[..end];

    if trimmed.ends_with(')') {
        let close_paren = previous_char_boundary(text, end)?;
        let open_paren = find_matching_open_paren(text, close_paren)?;
        let close_bracket = previous_char_boundary(text, open_paren)?;
        if char_at(text, close_bracket) != ']' {
            return None;
        }
        let open_bracket = find_matching_open_bracket(text, close_bracket)?;
        return Some(include_image_bang(text, open_bracket));
    }

    if trimmed.ends_with(']') {
        let close_reference = previous_char_boundary(text, end)?;
        let open_reference = find_matching_open_bracket(text, close_reference)?;
        let close_text = previous_char_boundary(text, open_reference)?;
        if char_at(text, close_text) != ']' {
            return None;
        }
        let open_text = find_matching_open_bracket(text, close_text)?;
        return Some(include_image_bang(text, open_text));
    }

    None
}

fn match_trailing_html_or_angle_start(text: &str, end: usize) -> Option<usize> {
    if !text[..end].ends_with('>') {
        return None;
    }

    if let Some((close_start, tag_name)) = trailing_html_closing_tag(text, end) {
        if let Some(open_start) = find_previous_opening_tag_start(text, close_start, &tag_name) {
            return Some(open_start);
        }
    }

    let start = text[..end].rfind('<')?;
    looks_like_complete_angle_construct(&text[start..end]).then_some(start)
}

fn match_trailing_backtick_construct_start(text: &str, end: usize) -> Option<usize> {
    let run_len = trailing_run_len(text, end, '`')?;
    let delimiter = "`".repeat(run_len);
    let close_start = end - run_len;

    for (candidate, _) in text[..close_start].rmatch_indices(&delimiter) {
        if !is_escaped_markdown_delimiter(text, candidate)
            && has_non_whitespace_content(text, candidate + run_len, close_start)
        {
            return Some(candidate);
        }
    }

    None
}

fn match_trailing_emphasis_start(text: &str, end: usize) -> Option<usize> {
    for delimiter in ["***", "___", "~~", "**", "__", "*", "_"] {
        if !text[..end].ends_with(delimiter) {
            continue;
        }

        let close_start = end - delimiter.len();
        if is_escaped_markdown_delimiter(text, close_start)
            || !can_close_emphasis(text, close_start, delimiter.len())
        {
            continue;
        }

        for (candidate, _) in text[..close_start].rmatch_indices(delimiter) {
            if !is_escaped_markdown_delimiter(text, candidate)
                && can_open_emphasis(text, candidate, delimiter.len())
                && has_non_whitespace_content(text, candidate + delimiter.len(), close_start)
            {
                return Some(candidate);
            }
        }
    }

    None
}

fn find_trailing_block_construct_start(text: &str) -> Option<usize> {
    let end = trim_trailing_whitespace(text, text.len());
    if end == 0 {
        return None;
    }

    let line_start = text[..end].rfind(['\n', '\r']).map_or(0, |idx| idx + 1);
    let line = &text[line_start..end];
    let trimmed = line.trim_start();

    if trimmed.starts_with('>') {
        return Some(line_start);
    }

    if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
        return Some(line_start);
    }

    if starts_with_ordered_list_marker(trimmed) || line.contains('|') {
        return Some(line_start);
    }

    None
}

fn find_trailing_delimiter_run_start(text: &str) -> Option<usize> {
    let end = trim_trailing_whitespace(text, text.len());
    let start = previous_char_boundary(text, end)?;
    let delimiter = char_at(text, start);
    if !matches!(delimiter, '`' | '*' | '_' | '~') {
        return None;
    }

    let run_len = trailing_run_len(text, end, delimiter)?;
    let run_start = end - run_len;
    (!is_escaped_markdown_delimiter(text, run_start)).then_some(run_start)
}

fn char_at(text: &str, idx: usize) -> char {
    text[idx..]
        .chars()
        .next()
        .expect("scan index should always be on a char boundary")
}

fn delimiter_run_len(text: &str, idx: usize, delimiter: char) -> usize {
    text[idx..]
        .chars()
        .take_while(|ch| *ch == delimiter)
        .map(char::len_utf8)
        .sum()
}

fn trailing_run_len(text: &str, end: usize, delimiter: char) -> Option<usize> {
    let mut current = end;
    let mut total = 0;

    while let Some(prev) = previous_char_boundary(text, current) {
        if char_at(text, prev) != delimiter {
            break;
        }
        total += delimiter.len_utf8();
        current = prev;
    }

    (total > 0).then_some(total)
}

fn previous_char_boundary(text: &str, idx: usize) -> Option<usize> {
    text[..idx]
        .char_indices()
        .next_back()
        .map(|(start, _)| start)
}

fn trim_trailing_whitespace(text: &str, mut end: usize) -> usize {
    while let Some((idx, ch)) = text[..end].char_indices().next_back() {
        if !ch.is_whitespace() {
            break;
        }
        end = idx;
    }

    end
}

fn has_non_whitespace_content(text: &str, start: usize, end: usize) -> bool {
    text[start..end].chars().any(|ch| !ch.is_whitespace())
}

fn starts_with_ordered_list_marker(line: &str) -> bool {
    let digits_len = line.chars().take_while(|ch| ch.is_ascii_digit()).count();
    digits_len > 0 && line[digits_len..].starts_with(". ")
}

fn include_image_bang(text: &str, open_bracket: usize) -> usize {
    previous_char_boundary(text, open_bracket)
        .filter(|prev| char_at(text, *prev) == '!')
        .unwrap_or(open_bracket)
}

fn find_matching_open_bracket(text: &str, close_idx: usize) -> Option<usize> {
    let mut depth = 1;
    let mut current = close_idx;

    while let Some(prev) = previous_char_boundary(text, current) {
        let ch = char_at(text, prev);
        if is_escaped_markdown_delimiter(text, prev) {
            current = prev;
            continue;
        }

        match ch {
            ']' => depth += 1,
            '[' => {
                depth -= 1;
                if depth == 0 {
                    return Some(prev);
                }
            }
            _ => {}
        }

        current = prev;
    }

    None
}

fn find_matching_open_paren(text: &str, close_idx: usize) -> Option<usize> {
    let mut depth = 1;
    let mut current = close_idx;

    while let Some(prev) = previous_char_boundary(text, current) {
        let ch = char_at(text, prev);
        if is_escaped_markdown_delimiter(text, prev) {
            current = prev;
            continue;
        }

        match ch {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    return Some(prev);
                }
            }
            _ => {}
        }

        current = prev;
    }

    None
}

fn find_matching_close_paren(text: &str, open_idx: usize) -> Option<usize> {
    let mut depth = 0;
    let mut current = open_idx;

    while current < text.len() {
        let ch = char_at(text, current);
        if !is_escaped_markdown_delimiter(text, current) {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(current);
                    }
                }
                _ => {}
            }
        }

        current += ch.len_utf8();
    }

    None
}

fn looks_like_angle_construct_start(text: &str, idx: usize) -> bool {
    let next = text[idx + char_at(text, idx).len_utf8()..].chars().next();
    matches!(
        next,
        Some(ch) if ch.is_ascii_alphabetic() || matches!(ch, '/' | '!' | '?')
    )
}

fn looks_like_complete_angle_construct(slice: &str) -> bool {
    if !slice.starts_with('<') || !slice.ends_with('>') {
        return false;
    }

    let inner = slice[1..slice.len() - 1].trim();
    if inner.is_empty() {
        return false;
    }

    inner.starts_with('/')
        || inner.starts_with('!')
        || inner.starts_with('?')
        || inner.starts_with("http://")
        || inner.starts_with("https://")
        || inner.starts_with("mailto:")
        || inner
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic())
        || (inner.contains('@') && !inner.chars().any(|ch| ch.is_whitespace()))
}

fn trailing_html_closing_tag(text: &str, end: usize) -> Option<(usize, String)> {
    let start = text[..end].rfind('<')?;
    let slice = &text[start..end];
    if !slice.starts_with("</") || !slice.ends_with('>') {
        return None;
    }

    let tag_name = slice[2..slice.len() - 1].trim();
    (!tag_name.is_empty()
        && tag_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-'))
    .then(|| (start, tag_name.to_owned()))
}

fn find_previous_opening_tag_start(text: &str, before_end: usize, tag_name: &str) -> Option<usize> {
    let pattern = format!("<{tag_name}");

    for (candidate, _) in text[..before_end].rmatch_indices(&pattern) {
        let suffix = &text[candidate + pattern.len()..before_end];
        let Some(next) = suffix.chars().next() else {
            continue;
        };
        if !(next == '>' || next == '/' || next.is_whitespace()) {
            continue;
        }

        let Some(close_rel) = suffix.find('>') else {
            continue;
        };
        let close_idx = candidate + pattern.len() + close_rel;
        if text[candidate..close_idx + 1].trim_end().ends_with("/>") {
            continue;
        }

        return Some(candidate);
    }

    None
}

fn is_escaped_markdown_delimiter(text: &str, idx: usize) -> bool {
    let slash_count = text[..idx]
        .chars()
        .rev()
        .take_while(|ch| *ch == '\\')
        .count();
    slash_count % 2 == 1
}

fn can_open_emphasis(text: &str, idx: usize, delimiter_len: usize) -> bool {
    let prev = text[..idx].chars().next_back();
    let next = text[idx + delimiter_len..].chars().next();
    matches!(next, Some(ch) if !ch.is_whitespace())
        && prev.is_none_or(|ch| ch.is_whitespace() || ch.is_ascii_punctuation())
}

fn can_close_emphasis(text: &str, idx: usize, delimiter_len: usize) -> bool {
    let prev = text[..idx].chars().next_back();
    let next = text[idx + delimiter_len..].chars().next();
    matches!(prev, Some(ch) if !ch.is_whitespace())
        && next.is_none_or(|ch| ch.is_whitespace() || ch.is_ascii_punctuation())
}

#[allow(dead_code)]
pub(crate) fn reconcile_streamed_response(
    stdout: &mut impl Write,
    streamed_text: &str,
    final_text: &str,
    _printed_any: bool,
    green_responses: bool,
    response_color_active: &mut bool,
) -> Result<()> {
    reconcile_streamed_response_with_pending(
        stdout,
        streamed_text,
        "",
        final_text,
        green_responses,
        response_color_active,
    )
}

fn reconcile_streamed_response_with_pending(
    stdout: &mut impl Write,
    printed_text: &str,
    pending_text: &str,
    final_text: &str,
    green_responses: bool,
    response_color_active: &mut bool,
) -> Result<()> {
    reconcile_streamed_response_with_rewrite_safety(
        stdout,
        printed_text,
        pending_text,
        final_text,
        green_responses,
        response_color_active,
        rewrite_response_line_is_safe(printed_text, final_text),
    )
}

fn reconcile_streamed_response_with_rewrite_safety(
    stdout: &mut impl Write,
    printed_text: &str,
    pending_text: &str,
    final_text: &str,
    green_responses: bool,
    response_color_active: &mut bool,
    rewrite_in_place_safe: bool,
) -> Result<()> {
    let printed_any = !printed_text.is_empty();
    if !printed_any {
        if !final_text.is_empty() {
            write_complete_response(stdout, final_text, green_responses)?;
        }
        return Ok(());
    }

    let effective_streamed_text = format!("{printed_text}{pending_text}");
    if let Some(suffix) = final_text.strip_prefix(printed_text)
        && (effective_streamed_text == final_text
            || final_text.starts_with(&effective_streamed_text))
    {
        if !suffix.is_empty() {
            write_response_chunk(stdout, suffix, green_responses, response_color_active)?;
        }
        return finish_terminal_stream(stdout, true, response_color_active);
    }

    if printed_text == final_text {
        return finish_terminal_stream(stdout, true, response_color_active);
    }

    if let Some(suffix) = final_text.strip_prefix(printed_text) {
        if !suffix.is_empty() {
            write_response_chunk(stdout, suffix, green_responses, response_color_active)?;
        }
        return finish_terminal_stream(stdout, true, response_color_active);
    }

    if rewrite_in_place_safe {
        return rewrite_response_line(
            stdout,
            printed_text,
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

fn rewrite_response_line_is_safe(streamed_text: &str, final_text: &str) -> bool {
    can_rewrite_response_line(streamed_text, final_text) && !cfg!(windows)
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
        windows_console_support.unwrap_or(false)
            || get("WT_SESSION").is_some()
            || get("ANSICON").is_some()
            || matches!(get("ConEmuANSI"), Some(value) if value.eq_ignore_ascii_case("ON"))
            || (!term.is_empty() && !term.eq_ignore_ascii_case("dumb"))
    }

    #[cfg(not(windows))]
    {
        windows_console_support.unwrap_or(!term.is_empty())
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

fn ask_progress_status_line(state: &shared_protocol::JobState) -> Option<JobStatusLine> {
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

    #[test]
    fn ask_progress_status_line_only_emits_provider_recovery_messages() {
        let state = shared_protocol::JobState {
            job_id: "job-1".to_owned(),
            status: shared_protocol::JobStatus::Running,
            message: Some("Reloading tab and starting a fresh conversation...".to_owned()),
            conversation_id: None,
            updated_at: chrono::Utc::now(),
            artifacts: vec![],
        };
        assert_eq!(
            ask_progress_status_line(&state),
            Some(JobStatusLine {
                status: "running".to_owned(),
                message: "Reloading tab and starting a fresh conversation...".to_owned(),
            })
        );

        let ignored = shared_protocol::JobState {
            job_id: "job-1".to_owned(),
            status: shared_protocol::JobStatus::Running,
            message: Some("drafting answer".to_owned()),
            conversation_id: None,
            updated_at: chrono::Utc::now(),
            artifacts: vec![],
        };
        assert_eq!(ask_progress_status_line(&ignored), None);
    }

    fn render_reconciled_response(
        streamed_chunks: &[&str],
        final_text: &str,
        green_responses: bool,
    ) -> String {
        let rewrite_in_place_safe = render_streamed_chunks(streamed_chunks, green_responses)
            .1
            .printed_text;
        render_reconciled_response_with_rewrite_safety(
            streamed_chunks,
            final_text,
            green_responses,
            rewrite_response_line_is_safe(&rewrite_in_place_safe, final_text),
        )
    }

    fn render_streamed_chunks(
        streamed_chunks: &[&str],
        green_responses: bool,
    ) -> (String, ResponseStreamState) {
        let mut stdout = Vec::new();
        let mut response_stream = ResponseStreamState::default();

        for chunk in streamed_chunks {
            buffer_response_chunk(&mut stdout, chunk, green_responses, &mut response_stream)
                .unwrap();
        }

        (String::from_utf8(stdout).unwrap(), response_stream)
    }

    fn render_reconciled_response_with_rewrite_safety(
        streamed_chunks: &[&str],
        final_text: &str,
        green_responses: bool,
        rewrite_in_place_safe: bool,
    ) -> String {
        let (streamed_output, mut response_stream) =
            render_streamed_chunks(streamed_chunks, green_responses);
        let mut stdout = streamed_output.into_bytes();

        reconcile_streamed_response_with_rewrite_safety(
            &mut stdout,
            &response_stream.printed_text,
            &response_stream.pending_text,
            final_text,
            green_responses,
            &mut response_stream.response_color_active,
            rewrite_in_place_safe,
        )
        .unwrap();

        String::from_utf8(stdout).unwrap()
    }

    fn assert_stream_split(chunks: &[&str], expected_output: &str, expected_pending: &str) {
        let (output, response_stream) = render_streamed_chunks(chunks, false);
        assert_eq!(output, expected_output);
        assert_eq!(response_stream.printed_text, expected_output);
        assert_eq!(response_stream.pending_text, expected_pending);
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
        if cfg!(windows) {
            return;
        }
        assert_eq!(
            render_reconciled_response(&["ASK_", "ASK_"], "ASK_OK", false),
            "ASK_ASK_\rASK_OK  \n"
        );
    }

    #[test]
    fn reconcile_streamed_response_rewrites_divergent_single_line_chunks() {
        if cfg!(windows) {
            return;
        }
        assert_eq!(
            render_reconciled_response(&["WRONG"], "ASK_OK", false),
            "WRONG\rASK_OK\n"
        );
    }

    #[test]
    fn reconcile_streamed_response_prints_final_output_when_no_chunks_stream() {
        assert_eq!(render_reconciled_response(&[], "ASK_OK", false), "ASK_OK\n");
    }

    #[test]
    fn reconcile_streamed_response_falls_back_when_rewrite_is_unsafe() {
        assert_eq!(
            render_reconciled_response_with_rewrite_safety(&["WRONG"], "ASK_OK", false, false),
            "WRONG\nASK_OK\n"
        );
    }

    #[test]
    fn reconcile_streamed_response_fallback_resets_green_output_before_printing_final_text() {
        assert_eq!(
            render_reconciled_response_with_rewrite_safety(&["WRONG"], "ASK_OK", true, false),
            format!("{ANSI_GREEN}WRONG{ANSI_RESET}\n{ANSI_GREEN}ASK_OK{ANSI_RESET}\n")
        );
    }

    #[test]
    fn buffer_response_chunk_holds_unstable_bold_suffix_until_closed() {
        let (output, response_stream) =
            render_streamed_chunks(&["The core tradeoff is this: **TypeScr"], false);
        assert_eq!(output, "The core tradeoff is this: ");
        assert_eq!(response_stream.printed_text, "The core tradeoff is this: ");
        assert_eq!(response_stream.pending_text, "**TypeScr");
    }

    #[test]
    fn buffer_response_chunk_holds_closed_trailing_markdown_constructs_in_pending_text() {
        for (input, expected_prefix, expected_pending) in [
            ("Lead `code`", "Lead ", "`code`"),
            (
                "Lead ```rust\nfn main() {}\n```",
                "Lead ",
                "```rust\nfn main() {}\n```",
            ),
            ("Lead *italic*", "Lead ", "*italic*"),
            ("Lead **bold**", "Lead ", "**bold**"),
            ("Lead ***both***", "Lead ", "***both***"),
            ("Lead ~~gone~~", "Lead ", "~~gone~~"),
            (
                "Lead [text](https://example.com)",
                "Lead ",
                "[text](https://example.com)",
            ),
            ("Lead [text][ref]", "Lead ", "[text][ref]"),
            ("Lead ![alt](image.png)", "Lead ", "![alt](image.png)"),
            (
                "Lead <https://example.com>",
                "Lead ",
                "<https://example.com>",
            ),
            ("Lead <tag>html</tag>", "Lead ", "<tag>html</tag>"),
            ("Lead <tag />", "Lead ", "<tag />"),
        ] {
            assert_stream_split(&[input], expected_prefix, expected_pending);
        }
    }

    #[test]
    fn buffer_response_chunk_preserves_plain_prefix_before_unstable_tail() {
        assert_stream_split(&["C++ is a **low-level**"], "C++ is a ", "**low-level**");
        assert_stream_split(&["Intro\n> quoted"], "Intro\n", "> quoted");
        assert_stream_split(&["Items:\n- pending"], "Items:\n", "- pending");
        assert_stream_split(&["Table:\n| a | b |"], "Table:\n", "| a | b |");
    }

    #[test]
    fn reconcile_streamed_response_prints_partial_bold_response_once_on_windows_safe_path() {
        let final_text =
            "The core tradeoff is this: **TypeScript optimizes for tool authoring speed**";
        assert_eq!(
            render_reconciled_response_with_rewrite_safety(
                &["The core tradeoff is this: **TypeScr"],
                final_text,
                false,
                false
            ),
            format!("{final_text}\n")
        );
    }

    #[test]
    fn reconcile_streamed_response_prints_revised_trailing_markdown_once_on_windows_safe_path() {
        let final_text = "C++ is a **low-level, high-performance** language";
        assert_eq!(
            render_reconciled_response_with_rewrite_safety(
                &["C++ is a **low-l**"],
                final_text,
                false,
                false
            ),
            format!("{final_text}\n")
        );
    }

    #[test]
    fn reconcile_streamed_response_flushes_completed_bold_token_without_duplicate_final_output() {
        let final_text =
            "The core tradeoff is this: **TypeScript optimizes for tool authoring speed**";
        assert_eq!(
            render_reconciled_response_with_rewrite_safety(
                &[
                    "The core tradeoff is this: **TypeScr",
                    "ipt optimizes for tool authoring speed**",
                ],
                final_text,
                false,
                false
            ),
            format!("{final_text}\n")
        );
    }

    #[test]
    fn buffer_response_chunk_releases_withheld_markdown_after_following_plain_text_arrives() {
        let (output, response_stream) =
            render_streamed_chunks(&["Answer: **bold**", " text"], false);
        assert_eq!(output, "Answer: **bold** text");
        assert_eq!(response_stream.printed_text, "Answer: **bold** text");
        assert!(response_stream.pending_text.is_empty());
    }

    #[test]
    fn buffer_response_chunk_withholds_only_the_unstable_tail_of_mixed_sequences() {
        assert_stream_split(&["Mix **bold** `code`"], "Mix ", "**bold** `code`");

        let (output, response_stream) =
            render_streamed_chunks(&["Mix **bold** `code`", " tail"], false);
        assert_eq!(output, "Mix **bold** `code` tail");
        assert_eq!(response_stream.printed_text, "Mix **bold** `code` tail");
        assert!(response_stream.pending_text.is_empty());
    }

    #[test]
    fn buffer_response_chunk_treats_nested_and_unmatched_delimiters_as_unstable() {
        for (input, expected_prefix, expected_pending) in [
            ("Edge **bold", "Edge ", "**bold"),
            ("Edge _italic", "Edge ", "_italic"),
            ("Edge ___combo", "Edge ", "___combo"),
            ("Edge ~~strike", "Edge ", "~~strike"),
            ("Edge `code", "Edge ", "`code"),
            ("Edge [link", "Edge ", "[link"),
            ("Edge [text](url", "Edge ", "[text](url"),
            ("Edge <tag", "Edge ", "<tag"),
            ("Edge **bold _italic", "Edge ", "**bold _italic"),
            ("Edge ~~**mix**", "Edge ", "~~**mix**"),
        ] {
            assert_stream_split(&[input], expected_prefix, expected_pending);
        }
    }

    #[test]
    fn plain_text_streaming_is_unchanged() {
        let (output, response_stream) = render_streamed_chunks(
            &["Plain text streams immediately.", " Still plain text."],
            false,
        );
        assert_eq!(output, "Plain text streams immediately. Still plain text.");
        assert_eq!(
            response_stream.printed_text,
            "Plain text streams immediately. Still plain text."
        );
        assert!(response_stream.pending_text.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn reconcile_streamed_response_falls_back_for_width_boundary_on_windows() {
        let streamed_text = format!("{}B", "A".repeat(79));
        let final_text = format!("{}C", "A".repeat(79));
        assert_eq!(
            render_reconciled_response(&[streamed_text.as_str()], &final_text, false),
            format!("{streamed_text}\n{final_text}\n")
        );
    }

    #[cfg(windows)]
    #[test]
    fn console_attributes_background_is_black_reads_console_background_bits() {
        assert!(console_attributes_background_is_black(0));
        assert!(!console_attributes_background_is_black(0x0010));
        assert!(!console_attributes_background_is_black(0x00F0));
    }
}
