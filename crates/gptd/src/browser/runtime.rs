use anyhow::{Context, Result, bail};
use shared_protocol::BrowserConnectionMode;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub(super) struct AdapterRuntime {
    pub entry: PathBuf,
    pub asset_root: PathBuf,
    pub launcher: AdapterLauncher,
}

pub(super) enum AdapterLauncher {
    Node,
    Tsx(PathBuf),
}

pub(crate) fn browser_integration_prereqs_available() -> bool {
    if std::env::var_os("TABLINQ_SKIP_BROWSER_INTEGRATION_TESTS").is_some() {
        return false;
    }

    if !command_available(node_command(), "--version") {
        return false;
    }

    let Ok(runtime) = resolve_adapter_runtime() else {
        return false;
    };

    has_playwright_core(&runtime.entry)
}

pub(super) fn browser_connection_mode_name(value: BrowserConnectionMode) -> &'static str {
    match value {
        BrowserConnectionMode::AttachFirst => "attach_first",
        BrowserConnectionMode::ManagedFallback => "managed_fallback",
        BrowserConnectionMode::ManagedOnly => "managed_only",
    }
}

pub(super) fn resolve_adapter_runtime() -> Result<AdapterRuntime> {
    if let Ok(value) = std::env::var("TABLINQ_BROWSER_ADAPTER_ENTRY") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let entry = PathBuf::from(trimmed);
            if entry.exists() {
                let asset_root = std::env::var("TABLINQ_ASSET_ROOT")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(PathBuf::from)
                    .or_else(|| infer_asset_root(&entry))
                    .unwrap_or_else(|| {
                        entry
                            .parent()
                            .unwrap_or_else(|| Path::new("."))
                            .to_path_buf()
                    });
                return build_adapter_runtime(entry, asset_root)
                    .context("resolve TABLINQ_BROWSER_ADAPTER_ENTRY");
            }
        }
    }

    let mut last_error = None;
    for (entry, asset_root) in adapter_runtime_candidates() {
        if entry.exists() {
            match build_adapter_runtime(entry, asset_root) {
                Ok(runtime) => return Ok(runtime),
                Err(err) => last_error = Some(err),
            }
        }
    }

    if let Some(err) = last_error {
        return Err(err).context("browser adapter entry could not be resolved");
    }

    bail!("browser adapter entry could not be resolved")
}

fn adapter_runtime_candidates() -> Vec<(PathBuf, PathBuf)> {
    let mut candidates = Vec::new();

    if let Ok(current_exe) = std::env::current_exe()
        && let Some(bin_dir) = current_exe.parent()
        && let Some(install_root) = bin_dir.parent()
    {
        candidates.push((
            install_root
                .join("browser-adapter")
                .join("src")
                .join("service.js"),
            install_root.to_path_buf(),
        ));
        candidates.push((
            install_root
                .join("packages")
                .join("browser-adapter")
                .join("dist")
                .join("src")
                .join("service.js"),
            install_root.to_path_buf(),
        ));
    }

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    candidates.push((
        repo_root
            .join("packages")
            .join("browser-adapter")
            .join("src")
            .join("service.ts"),
        repo_root.clone(),
    ));
    candidates.push((
        repo_root
            .join("packages")
            .join("browser-adapter")
            .join("dist")
            .join("src")
            .join("service.js"),
        repo_root.clone(),
    ));

    candidates
}

fn infer_asset_root(entry: &Path) -> Option<PathBuf> {
    entry.ancestors().find_map(|ancestor| {
        if ancestor.join("selector-packs").exists() || ancestor.join("packages").exists() {
            Some(ancestor.to_path_buf())
        } else {
            None
        }
    })
}

fn build_adapter_runtime(entry: PathBuf, asset_root: PathBuf) -> Result<AdapterRuntime> {
    let launcher = if is_js_entry(&entry) {
        AdapterLauncher::Node
    } else {
        AdapterLauncher::Tsx(resolve_tsx_runner(&entry).with_context(|| {
            format!(
                "TypeScript browser adapter entry {} requires a local tsx runner",
                entry.display()
            )
        })?)
    };

    Ok(AdapterRuntime {
        entry,
        asset_root,
        launcher,
    })
}

fn resolve_tsx_runner(entry: &Path) -> Option<PathBuf> {
    entry.ancestors().find_map(|ancestor| {
        let candidate = ancestor
            .join("node_modules")
            .join(".bin")
            .join(if cfg!(windows) { "tsx.cmd" } else { "tsx" });
        candidate.exists().then_some(candidate)
    })
}

fn has_playwright_core(entry: &Path) -> bool {
    entry.ancestors().any(|ancestor| {
        ancestor
            .join("node_modules")
            .join("playwright-core")
            .join("package.json")
            .exists()
    })
}

fn command_available(command: &str, version_arg: &str) -> bool {
    Command::new(command)
        .arg(version_arg)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub(super) fn node_command() -> &'static str {
    if cfg!(windows) { "node.exe" } else { "node" }
}

fn is_js_entry(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("js"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::adapter_runtime_candidates;

    #[test]
    fn repo_checkout_prefers_source_adapter_before_dist_output() {
        let candidates = adapter_runtime_candidates();
        let repo_tail = &candidates[candidates.len().saturating_sub(2)..];

        assert_eq!(repo_tail.len(), 2, "expected repo source/dist candidates");
        assert!(
            repo_tail[0]
                .0
                .ends_with("packages/browser-adapter/src/service.ts"),
            "repo checkout should prefer source adapter before stale dist output",
        );
        assert!(
            repo_tail[1]
                .0
                .ends_with("packages/browser-adapter/dist/src/service.js"),
            "repo checkout should keep dist output as the fallback candidate",
        );
    }
}
