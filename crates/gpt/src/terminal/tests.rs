use crate::CliExecution;
use serde_json::json;

#[cfg(windows)]
use super::ansi::console_attributes_background_is_black;
use super::ansi::{
    ANSI_GREEN, ANSI_RESET, ansi_color_supported, format_cli_error_with_color,
    shell_background_is_black,
};
use super::execution::{
    JobStatusLine, ask_progress_status_line, extract_research_plan, research_progress_status_line,
};
use super::response::{
    ResponseStreamState, buffer_response_chunk, reconcile_streamed_response_with_rewrite_safety,
    rewrite_response_line_is_safe,
};

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

#[test]
fn research_progress_status_line_keeps_research_output_minimal() {
    let queued = research_job_state(
        shared_protocol::JobStatus::Queued,
        Some("queued research job"),
    );
    assert_eq!(
        research_progress_status_line(&queued, false),
        Some(JobStatusLine {
            status: "queued".to_owned(),
            message: "queued research job".to_owned(),
        })
    );

    let waiting = research_job_state(
        shared_protocol::JobStatus::WaitingForSources,
        Some("awaiting start confirmation"),
    );
    assert_eq!(
        research_progress_status_line(&waiting, false),
        Some(JobStatusLine {
            status: "waitingforsources".to_owned(),
            message: "awaiting start confirmation".to_owned(),
        })
    );

    let running = research_job_state(
        shared_protocol::JobStatus::Running,
        Some("Research plan\nUpdate\nCollect sources.\nStop research"),
    );
    assert_eq!(
        research_progress_status_line(&running, false),
        Some(JobStatusLine {
            status: "running".to_owned(),
            message: "research started".to_owned(),
        })
    );
    assert_eq!(research_progress_status_line(&running, true), None);

    let noisy_running = research_job_state(
        shared_protocol::JobStatus::Running,
        Some("155 searches\n0\n1"),
    );
    assert_eq!(research_progress_status_line(&noisy_running, true), None);

    let failed = research_job_state(shared_protocol::JobStatus::Failed, Some("adapter failed"));
    assert_eq!(
        research_progress_status_line(&failed, true),
        Some(JobStatusLine {
            status: "failed".to_owned(),
            message: "adapter failed".to_owned(),
        })
    );
}

#[test]
fn extract_research_plan_omits_controls_and_activity_status() {
    let plan = extract_research_plan(
        "Geography comparative brief\n\
         Update\n\
         Extract and validate city facts from the attached dataset and metadata.\n\
         Collect authoritative geography, climate, and hazard data for each city.\n\
         Identify contradictions and quantify uncertainty across sources.\n\
         Rank cities by geography-driven infrastructure difficulty with justification.\n\
         Draft concise brief including summary, table, risks, confidence, and questions.\n\
         \n\
         Citing file content from uploaded materials...\n\
         \n\
         Stop research",
    );

    assert_eq!(
        plan.as_deref(),
        Some(
            "Extract and validate city facts from the attached dataset and metadata.\n\
             Collect authoritative geography, climate, and hazard data for each city.\n\
             Identify contradictions and quantify uncertainty across sources.\n\
             Rank cities by geography-driven infrastructure difficulty with justification.\n\
             Draft concise brief including summary, table, risks, confidence, and questions."
        )
    );
}

fn research_job_state(
    status: shared_protocol::JobStatus,
    message: Option<&str>,
) -> shared_protocol::JobState {
    shared_protocol::JobState {
        job_id: "job-1".to_owned(),
        status,
        message: message.map(str::to_owned),
        conversation_id: None,
        updated_at: chrono::Utc::now(),
        artifacts: vec![],
    }
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
        buffer_response_chunk(&mut stdout, chunk, green_responses, &mut response_stream).unwrap();
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

fn render_reconciled_research_response(streamed_chunks: &[&str], final_text: &str) -> String {
    render_reconciled_response_with_rewrite_safety(streamed_chunks, final_text, false, false)
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
    assert_eq!(
        render_reconciled_response_with_rewrite_safety(&["ASKX", "ASKX"], "ASK_OK", false, true),
        "ASKXASKX\rASK_OK  \n"
    );
}

#[test]
fn reconcile_streamed_response_rewrites_divergent_single_line_chunks() {
    assert_eq!(
        render_reconciled_response_with_rewrite_safety(&["WRONG"], "ASK_OK", false, true),
        "WRONG\rASK_OK\n"
    );
}

#[test]
fn reconcile_streamed_response_prints_final_output_when_no_chunks_stream() {
    assert_eq!(render_reconciled_response(&[], "ASK_OK", false), "ASK_OK\n");
}

#[test]
fn research_reconciliation_prints_withheld_markdown_tail_once() {
    let final_text = "Research result: **stable tail**";
    assert_eq!(
        render_reconciled_research_response(&["Research result: **stable"], final_text),
        format!("{final_text}\n")
    );
}

#[test]
fn research_reconciliation_appends_final_suffix_once() {
    assert_eq!(
        render_reconciled_research_response(&["Research prefix"], "Research prefix suffix"),
        "Research prefix suffix\n"
    );
}

#[test]
fn research_reconciliation_falls_back_without_rewriting_divergent_output() {
    assert_eq!(
        render_reconciled_research_response(&["Draft research"], "Final research"),
        "Draft research\nFinal research\n"
    );
}

#[test]
fn research_reconciliation_prints_final_report_once_without_terminal_chunks() {
    assert_eq!(
        render_reconciled_research_response(&[], "Complete research report"),
        "Complete research report\n"
    );
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
    assert_stream_split(
        &["Intro\n> quoted **tail**"],
        "Intro\n> quoted ",
        "**tail**",
    );
    assert_stream_split(
        &["Items:\n- pending `code`"],
        "Items:\n- pending ",
        "`code`",
    );
}

#[test]
fn buffer_response_chunk_streams_completed_block_lines_immediately() {
    for input in [
        "Intro\n> quoted",
        "Items:\n- pending",
        "Table:\n| a | b |",
        "Outline:\n1. numbered",
        "# C vs C++ vs C#\n\n> ## At a glance\n>\n> - **C** -> minimal, procedural, close to hardware",
    ] {
        assert_stream_split(&[input], input, "");
    }
}

#[test]
fn buffer_response_chunk_treats_whitespace_after_trailing_construct_as_unstable_tail() {
    assert_stream_split(&["Lead **bold**   "], "Lead **bold", "**   ");
}

#[test]
fn buffer_response_chunk_streams_newline_terminated_trailing_constructs() {
    assert_stream_split(
        &["Lead [link](https://example.com)\n"],
        "Lead [link](https://example.com)\n",
        "",
    );
}

#[test]
fn buffer_response_chunk_ignores_escaped_delimiters_and_non_construct_angles() {
    assert_stream_split(
        &[r"Literal \*asterisks\* stay plain"],
        r"Literal \*asterisks\* stay plain",
        "",
    );
    assert_stream_split(
        &[r"Literal \`code\` stays plain"],
        r"Literal \`code\` stays plain",
        "",
    );
    assert_stream_split(&["Math: 2 < 3 > 1"], "Math: 2 < 3 > 1", "");
}

#[test]
fn reconcile_streamed_response_prints_partial_bold_response_once_on_windows_safe_path() {
    let final_text = "The core tradeoff is this: **TypeScript optimizes for tool authoring speed**";
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
fn reconcile_streamed_response_finishes_pending_construct_without_reprinting() {
    let final_text = "Answer: **bold**";
    assert_eq!(
        render_reconciled_response_with_rewrite_safety(
            &["Answer: **bold**"],
            final_text,
            false,
            false
        ),
        format!("{final_text}\n")
    );
}

#[test]
fn reconcile_streamed_response_finishes_pending_construct_with_following_whitespace_once() {
    let final_text = "Answer: **bold**   ";
    assert_eq!(
        render_reconciled_response_with_rewrite_safety(
            &["Answer: **bold**   "],
            final_text,
            false,
            false
        ),
        format!("{final_text}\n")
    );
}

#[test]
fn reconcile_streamed_response_flushes_completed_bold_token_without_duplicate_final_output() {
    let final_text = "The core tradeoff is this: **TypeScript optimizes for tool authoring speed**";
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
    let (output, response_stream) = render_streamed_chunks(&["Answer: **bold**", " text"], false);
    assert_eq!(output, "Answer: **bold** text");
    assert_eq!(response_stream.printed_text, "Answer: **bold** text");
    assert!(response_stream.pending_text.is_empty());
}

#[test]
fn buffer_response_chunk_releases_multiple_adjacent_withheld_constructs_together() {
    let (output, response_stream) = render_streamed_chunks(
        &["Mix **bold** `code` [link](https://example.com)", " tail"],
        false,
    );
    assert_eq!(
        output,
        "Mix **bold** `code` [link](https://example.com) tail"
    );
    assert_eq!(
        response_stream.printed_text,
        "Mix **bold** `code` [link](https://example.com) tail"
    );
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
        ("Edge [text][ref", "Edge [text]", "[ref"),
        ("Edge ![alt](image", "Edge ", "![alt](image"),
        ("Edge <https://example.com", "Edge ", "<https://example.com"),
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
