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

pub(super) fn write_response_chunk(
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

pub(super) fn write_complete_response(
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

pub(super) fn buffer_response_chunk(
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

#[allow(dead_code)]
pub(super) fn reconcile_streamed_response(
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

pub(super) fn reconcile_streamed_response_with_pending(
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

pub(super) fn reconcile_streamed_response_with_rewrite_safety(
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
