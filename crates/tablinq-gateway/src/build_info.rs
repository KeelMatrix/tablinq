use shared_protocol::BuildMetadata;

pub fn build_metadata() -> BuildMetadata {
    BuildMetadata {
        application: "tablinq-gateway".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        git_commit: env_commit(),
        git_dirty: option_env!("TABLINQ_BUILD_DIRTY").unwrap_or("false") == "true",
        target: option_env!("TABLINQ_BUILD_TARGET")
            .unwrap_or(std::env::consts::ARCH)
            .to_owned(),
        profile: option_env!("TABLINQ_BUILD_PROFILE")
            .unwrap_or("dev")
            .to_owned(),
        built_at_unix_epoch: option_env!("TABLINQ_BUILD_UNIX_EPOCH")
            .unwrap_or("0")
            .parse()
            .unwrap_or(0),
    }
}

pub fn render(json: bool) -> String {
    let metadata = build_metadata();
    if json {
        serde_json::to_string_pretty(&metadata).expect("serialize build metadata")
    } else {
        format!(
            "{app} {version} commit={commit} dirty={dirty} target={target} profile={profile} built_at_unix_epoch={built_at}",
            app = metadata.application,
            version = metadata.version,
            commit = metadata.git_commit.as_deref().unwrap_or("unknown"),
            dirty = metadata.git_dirty,
            target = metadata.target,
            profile = metadata.profile,
            built_at = metadata.built_at_unix_epoch,
        )
    }
}

fn env_commit() -> Option<String> {
    let value = option_env!("TABLINQ_BUILD_COMMIT")?;
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed == "unknown" {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
