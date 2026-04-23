#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DelimiterRun {
    start: usize,
    len: usize,
    ch: char,
}

pub(super) fn find_unstable_suffix_start(text: &str) -> Option<usize> {
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
