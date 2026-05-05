use anyhow::Result;
use std::io::Write;

use super::ansi::{ANSI_GREEN, ANSI_RESET};
use super::markdown::find_unstable_suffix_start;

#[derive(Debug, Default)]
pub(super) struct ResponseStreamState {
    pub(super) printed_text: String,
    pub(super) pending_text: String,
    pub(super) printed_any: bool,
    pub(super) response_color_active: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ResponseDisplay {
    Raw,
    InteractiveTerminal,
}

pub(super) struct ResponseReconciliation<'a> {
    pub(super) printed_text: &'a str,
    pub(super) pending_text: &'a str,
    pub(super) final_text: &'a str,
    pub(super) green_responses: bool,
    pub(super) display: ResponseDisplay,
    pub(super) rewrite_in_place_safe: bool,
}

pub(super) fn write_response_chunk(
    stdout: &mut impl Write,
    text: &str,
    green_responses: bool,
    display: ResponseDisplay,
    response_color_active: &mut bool,
) -> Result<()> {
    if green_responses && !*response_color_active {
        write!(stdout, "{ANSI_GREEN}")?;
        *response_color_active = true;
    }
    write!(stdout, "{}", display_response_text(text, display))?;
    stdout.flush()?;
    Ok(())
}

pub(super) fn write_complete_response(
    stdout: &mut impl Write,
    text: &str,
    green_responses: bool,
    display: ResponseDisplay,
) -> Result<()> {
    let text = display_response_text(text, display);
    if green_responses {
        writeln!(stdout, "{ANSI_GREEN}{text}{ANSI_RESET}")?;
    } else {
        writeln!(stdout, "{text}")?;
    }
    stdout.flush()?;
    Ok(())
}

pub(super) fn buffer_response_chunk(
    stdout: &mut impl Write,
    text: &str,
    green_responses: bool,
    display: ResponseDisplay,
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
        display,
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

#[allow(dead_code)]
pub(super) fn reconcile_streamed_response(
    stdout: &mut impl Write,
    streamed_text: &str,
    final_text: &str,
    _printed_any: bool,
    green_responses: bool,
    display: ResponseDisplay,
    response_color_active: &mut bool,
) -> Result<()> {
    reconcile_streamed_response_with_pending(
        stdout,
        streamed_text,
        "",
        final_text,
        green_responses,
        display,
        response_color_active,
    )
}

pub(super) fn reconcile_streamed_response_with_pending(
    stdout: &mut impl Write,
    printed_text: &str,
    pending_text: &str,
    final_text: &str,
    green_responses: bool,
    display: ResponseDisplay,
    response_color_active: &mut bool,
) -> Result<()> {
    let rewrite_in_place_safe = rewrite_response_line_is_safe(printed_text, final_text);
    reconcile_streamed_response_with_rewrite_safety(
        stdout,
        ResponseReconciliation {
            printed_text,
            pending_text,
            final_text,
            green_responses,
            display,
            rewrite_in_place_safe,
        },
        response_color_active,
    )
}

pub(super) fn reconcile_streamed_response_with_rewrite_safety(
    stdout: &mut impl Write,
    reconciliation: ResponseReconciliation<'_>,
    response_color_active: &mut bool,
) -> Result<()> {
    let ResponseReconciliation {
        printed_text,
        pending_text,
        final_text,
        green_responses,
        display,
        rewrite_in_place_safe,
    } = reconciliation;

    let printed_any = !printed_text.is_empty();
    if !printed_any {
        if !final_text.is_empty() {
            write_complete_response(stdout, final_text, green_responses, display)?;
        }
        return Ok(());
    }

    let effective_streamed_text = format!("{printed_text}{pending_text}");
    if let Some(suffix) = final_text.strip_prefix(printed_text)
        && (effective_streamed_text == final_text
            || final_text.starts_with(&effective_streamed_text))
    {
        if !suffix.is_empty() {
            write_response_chunk(
                stdout,
                suffix,
                green_responses,
                display,
                response_color_active,
            )?;
        }
        return finish_terminal_stream(stdout, true, response_color_active);
    }

    if printed_text == final_text {
        return finish_terminal_stream(stdout, true, response_color_active);
    }

    if let Some(suffix) = final_text.strip_prefix(printed_text) {
        if !suffix.is_empty() {
            write_response_chunk(
                stdout,
                suffix,
                green_responses,
                display,
                response_color_active,
            )?;
        }
        return finish_terminal_stream(stdout, true, response_color_active);
    }

    if rewrite_in_place_safe {
        return rewrite_response_line(
            stdout,
            printed_text,
            final_text,
            green_responses,
            display,
            response_color_active,
        );
    }

    finish_terminal_stream(stdout, true, response_color_active)?;
    if !final_text.is_empty() {
        write_complete_response(stdout, final_text, green_responses, display)?;
    }
    Ok(())
}

pub(super) fn rewrite_response_line_is_safe(streamed_text: &str, final_text: &str) -> bool {
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
    display: ResponseDisplay,
    response_color_active: &mut bool,
) -> Result<()> {
    if *response_color_active {
        write!(stdout, "{ANSI_RESET}")?;
        *response_color_active = false;
    }

    let streamed_display_text = display_response_text(streamed_text, display);
    let final_display_text = display_response_text(final_text, display);
    let clear_width = streamed_display_text
        .chars()
        .count()
        .saturating_sub(final_display_text.chars().count());
    let clear_padding = " ".repeat(clear_width);
    if green_responses {
        writeln!(
            stdout,
            "\r{ANSI_GREEN}{final_display_text}{ANSI_RESET}{clear_padding}"
        )?;
    } else {
        writeln!(stdout, "\r{final_display_text}{clear_padding}")?;
    }
    stdout.flush()?;
    Ok(())
}

pub(super) fn finish_terminal_stream(
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

fn display_response_text(text: &str, display: ResponseDisplay) -> String {
    match display {
        ResponseDisplay::Raw => text.to_owned(),
        ResponseDisplay::InteractiveTerminal => sanitize_interactive_response_text(text),
    }
}

fn sanitize_interactive_response_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\u{0}' => output.push_str("<NUL>"),
            '\u{7}' => output.push_str("<BEL>"),
            '\u{8}' => output.push_str("<BS>"),
            '\t' | '\n' => output.push(ch),
            '\r' => output.push_str("<CR>"),
            '\u{1b}' => output.push_str("<ESC>"),
            '\u{80}'..='\u{9f}' => {
                output.push_str(&format!("<U+{:04X}>", ch as u32));
            }
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {
                output.push_str(&format!("<U+{:04X}>", ch as u32));
            }
            _ if ch.is_control() => {
                output.push_str(&format!("<U+{:04X}>", ch as u32));
            }
            _ => output.push(ch),
        }
    }
    output
}
