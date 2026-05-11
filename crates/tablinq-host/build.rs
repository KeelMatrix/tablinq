use std::env;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    println!("cargo:rerun-if-env-changed=TABLINQ_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=TABLINQ_BUILD_DIRTY");
    emit_git_rerun_directives();

    println!(
        "cargo:rustc-env=TABLINQ_BUILD_COMMIT={}",
        build_commit().unwrap_or_else(|| "unknown".to_owned())
    );
    println!(
        "cargo:rustc-env=TABLINQ_BUILD_DIRTY={}",
        if git_dirty().unwrap_or(false) {
            "true"
        } else {
            "false"
        }
    );
    println!(
        "cargo:rustc-env=TABLINQ_BUILD_TARGET={}",
        env::var("TARGET").unwrap_or_else(|_| "unknown".to_owned())
    );
    println!(
        "cargo:rustc-env=TABLINQ_BUILD_PROFILE={}",
        env::var("PROFILE").unwrap_or_else(|_| "unknown".to_owned())
    );
    println!(
        "cargo:rustc-env=TABLINQ_BUILD_UNIX_EPOCH={}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs().to_string())
            .unwrap_or_else(|_| "0".to_owned())
    );
}

fn emit_git_rerun_directives() {
    for path_name in ["HEAD", "packed-refs"] {
        if let Some(path) = git_path(path_name) {
            println!("cargo:rerun-if-changed={}", path);
        }
    }

    if let Some(reference) = current_head_reference()
        && let Some(path) = git_path(&reference)
    {
        println!("cargo:rerun-if-changed={}", path);
    }
}

fn git_path(path: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-path", path])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let path = String::from_utf8(output.stdout).ok()?;
    let trimmed = path.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn current_head_reference() -> Option<String> {
    let output = Command::new("git")
        .args(["symbolic-ref", "-q", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let reference = String::from_utf8(output.stdout).ok()?;
    let trimmed = reference.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn build_commit() -> Option<String> {
    if let Ok(value) = env::var("TABLINQ_BUILD_COMMIT") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_owned());
        }
    }

    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let commit = String::from_utf8(output.stdout).ok()?;
    let trimmed = commit.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn git_dirty() -> Option<bool> {
    if let Ok(value) = env::var("TABLINQ_BUILD_DIRTY") {
        return Some(matches!(value.trim(), "1" | "true" | "yes"));
    }

    let status = Command::new("git")
        .args(["diff", "--quiet", "--ignore-submodules", "HEAD", "--"])
        .status()
        .ok()?;
    Some(!status.success())
}
