use url::Url;

pub(super) fn same_target_url(current_url: &str, target_url: &str) -> bool {
    let Ok(current) = Url::parse(current_url.trim()) else {
        return current_url.trim().trim_end_matches('/') == target_url.trim().trim_end_matches('/');
    };
    let Ok(target) = Url::parse(target_url.trim()) else {
        return current_url.trim().trim_end_matches('/') == target_url.trim().trim_end_matches('/');
    };
    if current.scheme() != target.scheme() || current.host_str() != target.host_str() {
        return false;
    }
    match conversation_id(&target) {
        Some(target_conversation) => conversation_id(&current) == Some(target_conversation),
        None => workspace_path(&current) == workspace_path(&target),
    }
}

fn conversation_id(url: &Url) -> Option<&str> {
    let segments = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())?;
    if segments.len() >= 2 && segments[segments.len() - 2].eq_ignore_ascii_case("c") {
        segments.last().copied().filter(|value| !value.is_empty())
    } else {
        None
    }
}

fn workspace_path(url: &Url) -> String {
    let mut segments = url
        .path_segments()
        .map(|segments| segments.collect::<Vec<_>>())
        .unwrap_or_default();
    if segments.len() >= 2 && segments[segments.len() - 2].eq_ignore_ascii_case("c") {
        segments.truncate(segments.len() - 2);
    }
    if segments.is_empty() {
        "/".to_owned()
    } else {
        format!("/{}", segments.join("/"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_target_url_matches_conversation_urls_by_id() {
        assert!(same_target_url(
            "https://chatgpt.com/g/g-workspace/c/abc-123",
            "https://chatgpt.com/c/abc-123"
        ));
        assert!(!same_target_url(
            "https://chatgpt.com/g/g-workspace/c/abc-123",
            "https://chatgpt.com/c/other"
        ));
    }

    #[test]
    fn same_target_url_matches_workspace_paths_when_target_has_no_conversation() {
        assert!(same_target_url(
            "https://chatgpt.com/g/g-workspace/c/abc-123",
            "https://chatgpt.com/g/g-workspace"
        ));
        assert!(!same_target_url(
            "https://chatgpt.com/g/other/c/abc-123",
            "https://chatgpt.com/g/g-workspace"
        ));
    }

    #[test]
    fn same_target_url_preserves_current_trailing_slash_behavior() {
        assert!(!same_target_url(
            "https://chatgpt.com/g/g-workspace/",
            "https://chatgpt.com/g/g-workspace"
        ));
        assert!(same_target_url("not a url/", "not a url"));
        assert!(!same_target_url("not a url", "different"));
    }

    #[test]
    fn same_target_url_rejects_different_scheme_or_host() {
        assert!(!same_target_url(
            "http://chatgpt.com/",
            "https://chatgpt.com/"
        ));
        assert!(!same_target_url(
            "https://example.com/",
            "https://chatgpt.com/"
        ));
    }
}
