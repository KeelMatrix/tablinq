use super::*;
use gptd::Database;
use gptd::browser::browser_integration_prereqs_available;
use std::sync::{Arc, Mutex};
use tempfile::{TempDir, tempdir};

mod support;
use support::*;

macro_rules! require_browser_integration_prereqs {
    () => {
        if !browser_integration_prereqs_available() {
            eprintln!(
                "skipping browser-backed gateway test: browser adapter prerequisites are unavailable"
            );
            return;
        }
    };
}

mod capabilities_provider;

mod chat_requests;

mod conversation_inspection;

mod failure_recovery_status;

mod gateway_process;

mod request_parsing;

mod scheduler_dsl;

mod scheduler_process_control;

mod uploads_artifacts;
