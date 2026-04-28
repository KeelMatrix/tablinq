#![allow(unused_imports)]
use super::super::{
    CoreService, ExecutionRecoveryState, recovery::is_provider_generation_failure_message,
};
use crate::browser::{
    BrowserDiagnosticsSnapshot, BrowserManager, RequestLifecycleUpdate, ResearchUpdate,
};
use crate::config::AppConfig;
use crate::db::Database;
use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use shared_protocol::{
    BrowserConnectionMode, BrowserStatusSnapshot, ErrorCode, EventPayload, HealthCheck,
    HealthStatus, JobState, JobStatus, ResearchCommand, SelectorPackSummary, StreamChunk,
    StreamChunkKind,
};
use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use url::Url;

use super::locks::browser_busy_message;

pub(in crate::core) fn busy_health_snapshot() -> HealthCheck {
    HealthCheck {
        status: HealthStatus::Degraded,
        checked_at: Utc::now(),
        browser_connected: false,
        extension_connected: false,
        authenticated: false,
        details: Some(browser_busy_message().to_owned()),
    }
}

pub(in crate::core) fn health_snapshot(
    config: &AppConfig,
    browser: BrowserStatusSnapshot,
) -> HealthCheck {
    let status = if !browser.running || !browser.extension_connected || !browser.authenticated {
        HealthStatus::Degraded
    } else {
        HealthStatus::Ok
    };
    HealthCheck {
        status,
        checked_at: Utc::now(),
        browser_connected: browser.running,
        extension_connected: browser.extension_connected,
        authenticated: browser.authenticated,
        details: Some(if config.warm_mode() {
            format!(
                "control plane ready; warm mode enabled; attach_mode={:?}; browser_running={}; extension_connected={}",
                browser.attach_mode, browser.running, browser.extension_connected
            )
        } else {
            browser_health_details(&browser, config)
        }),
    }
}

pub(in crate::core) fn recovery_guidance(
    config: &AppConfig,
    browser: &BrowserStatusSnapshot,
    diagnostics: Option<&BrowserDiagnosticsSnapshot>,
) -> Vec<String> {
    let mut guidance = Vec::new();
    let mut attach_problem = false;
    let debug_endpoint = browser
        .debug_endpoint
        .clone()
        .unwrap_or_else(|| config.browser_debug_endpoint());
    if config.safe_mode() {
        guidance.push(
            "safe mode is enabled; ask and research are disabled until config.toml is updated"
                .to_owned(),
        );
    }
    if is_attach_primary_mode(browser.attach_mode) {
        if !browser.running {
            attach_problem = true;
            if browser.action.as_deref() == Some("detached") {
                let browser_label = recovery_browser_label(config, browser);
                guidance
                    .push("Tablinq is detached from the attach-first browser session.".to_owned());
                guidance.push(format!(
                    "If {browser_label} is still open, run `gpt browser attach`."
                ));
                guidance.push(format!(
                    "If attach fails or the debug endpoint is gone, fully close {} and run `gpt browser start-debug`.",
                    browser_label
                ));
            } else {
                guidance.push(format!(
                    "attach endpoint is not reachable at {debug_endpoint}; fully close {}, run `gpt browser start-debug`, then rerun `gpt browser attach`",
                    recovery_browser_label(config, browser)
                ));
                guidance.push(
                    "run `gpt browser connection-info` and confirm the configured debug endpoint, target URL, and browser path before switching modes"
                        .to_owned(),
                );
            }
        }
        if browser.extension_required && !browser.extension_connected {
            attach_problem = true;
            guidance.push(
                "the Tablinq extension is missing or disconnected in the attached browser profile; confirm it is installed, enabled, and running in the same browser profile Tablinq is attaching to"
                    .to_owned(),
            );
        }
        if current_url_requires_login_or_challenge(browser.current_url.as_deref()) {
            attach_problem = true;
            guidance.push(
                "the attached ChatGPT tab is blocked on sign-in or a challenge page; complete that page in your normal browser, then rerun `gpt browser attach`"
                    .to_owned(),
            );
        } else if !browser.authenticated {
            attach_problem = true;
            guidance.push(
                "the attached ChatGPT tab is signed out; sign in to ChatGPT in your normal browser, then rerun `gpt browser attach`"
                    .to_owned(),
            );
        }
        if !browser.target_tab_found {
            attach_problem = true;
            guidance.push(
                "target tab not found; run `gpt browser open` to create or focus the configured ChatGPT tab after the browser is signed in"
                    .to_owned(),
            );
        }
        if browser.running
            && (!browser.target_tab_found
                || !browser.authenticated
                || (browser.extension_required && !browser.extension_connected))
        {
            guidance.push(
                "if multiple ChatGPT tabs are open, keep only the signed-in tab with the working Tablinq extension, then rerun `gpt browser attach`"
                    .to_owned(),
            );
        }
        if attach_problem {
            guidance.push(
                "only if your machine or browser profile cannot expose the local debug endpoint, or you need debugging or CI/local harness isolation, set `browser_connection_mode = \"managed_only\"` in config.toml and use `gpt browser open` as explicit fallback"
                    .to_owned(),
            );
        }
        if browser.running
            && browser.target_tab_found
            && browser.authenticated
            && (!browser.extension_required || browser.extension_connected)
        {
            guidance.push("attached browser session is ready".to_owned());
        }
    } else {
        if !browser.running {
            guidance.push(
                "managed browser fallback is fallback/debug-only; use `gpt browser open` to start the managed browser tab"
                    .to_owned(),
            );
        } else {
            guidance.push(
                "daemon can recreate the managed fallback tab and relaunch the browser adapter when the page crashes or closes"
                    .to_owned(),
            );
        }
        guidance.push(
            "use managed fallback for debugging, CI/local harness isolation, or when attach-first cannot expose a local debug endpoint"
                .to_owned(),
        );
        if !browser.authenticated {
            guidance.push(
                "managed browser fallback is signed out; open the managed browser and sign in before running ask or research"
                    .to_owned(),
            );
        }
        if !browser.extension_connected {
            guidance.push(
                "extension probe is disconnected in managed fallback mode; reload the page or relaunch the browser to refresh capabilities"
                    .to_owned(),
            );
        }
    }
    if let Some(diagnostics) = diagnostics {
        if !diagnostics.browser_detected {
            guidance.push(
                "no supported browser was detected; set an explicit browser path in config.toml"
                    .to_owned(),
            );
        }
        if !diagnostics.profile_accessible && supports_managed_profile_reset(browser.attach_mode) {
            guidance.push(
                "browser profile is not writable; close other managed browser instances or run `gpt browser reset-profile`"
                    .to_owned(),
            );
        }
    }
    guidance
}

fn recovery_browser_label(config: &AppConfig, browser: &BrowserStatusSnapshot) -> &'static str {
    let configured = config.browser_channel().to_ascii_lowercase();
    let actual = browser.browser_kind.to_ascii_lowercase();
    if configured == "msedge"
        || configured == "edge"
        || actual == "msedge"
        || actual.contains("edge")
    {
        "Edge"
    } else {
        "Chrome"
    }
}

pub(in crate::core) fn current_url_requires_login_or_challenge(current_url: Option<&str>) -> bool {
    let Some(current_url) = current_url else {
        return false;
    };
    let lowered = current_url.to_ascii_lowercase();
    ["challenge", "captcha", "login", "signin", "auth", "cf_chl"]
        .iter()
        .any(|marker| lowered.contains(marker))
}

fn browser_health_details(browser: &BrowserStatusSnapshot, config: &AppConfig) -> String {
    if !browser.running {
        return if is_attach_primary_mode(browser.attach_mode) {
            format!(
                "control plane ready; attach-first browser not connected; debug_endpoint={}",
                browser
                    .debug_endpoint
                    .clone()
                    .unwrap_or_else(|| config.browser_debug_endpoint())
            )
        } else {
            "control plane ready; managed browser fallback not started yet".to_owned()
        };
    }

    if is_attach_primary_mode(browser.attach_mode) {
        if !browser.target_tab_found {
            return "control plane ready; attached browser is connected but the target tab was not found"
                .to_owned();
        }
        return match browser.current_url.as_deref() {
            Some(url) => format!(
                "control plane ready; attached tab at {url}; authenticated={}; extension_connected={}",
                browser.authenticated, browser.extension_connected
            ),
            None => format!(
                "control plane ready; attached browser is connected; authenticated={}; extension_connected={}",
                browser.authenticated, browser.extension_connected
            ),
        };
    }

    match browser.current_url.as_deref() {
        Some(url) => format!(
            "control plane ready; managed fallback tab at {url}; extension_connected={}",
            browser.extension_connected
        ),
        None => "control plane ready; managed browser fallback is connected".to_owned(),
    }
}

pub(in crate::core) fn is_attach_primary_mode(mode: BrowserConnectionMode) -> bool {
    matches!(
        mode,
        BrowserConnectionMode::AttachFirst | BrowserConnectionMode::ManagedFallback
    )
}

pub(in crate::core) fn supports_managed_profile_reset(mode: BrowserConnectionMode) -> bool {
    matches!(
        mode,
        BrowserConnectionMode::ManagedFallback | BrowserConnectionMode::ManagedOnly
    )
}

pub(in crate::core) fn default_selector_pack_summary() -> Option<SelectorPackSummary> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("selector-packs")
        .join("v1")
        .join("chatgpt-default.json");
    let raw = fs::read_to_string(&path).ok()?;
    let parsed: shared_protocol::SelectorPack = serde_json::from_str(&raw).ok()?;
    Some(SelectorPackSummary {
        pack_id: parsed.pack_id,
        version: format!("v{}", parsed.format_version),
        updated_at: Some(parsed.updated_at),
        path: Some(path.display().to_string()),
    })
}
