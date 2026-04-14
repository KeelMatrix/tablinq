# Core Audit

This note captures the current separation of concerns now that Tablinq has a browser-backed provider gateway, durable request lifecycle state, and interruption recovery.

## Current separation

- Core execution engine: [`crates/gptd/src/core.rs`](/C:/Users/user/Desktop/gpt-cli/crates/gptd/src/core.rs)
  - owns ask and research execution
  - owns request lifecycle transitions and recovery markers
  - owns browser lifecycle calls
  - owns job, session, conversation, artifact, request-summary, and failure-bundle persistence
  - calls the Playwright and extension runtime only through [`BrowserManager`](/C:/Users/user/Desktop/gpt-cli/crates/gptd/src/browser.rs)
- Admin CLI: [`crates/gpt/src/lib.rs`](/C:/Users/user/Desktop/gpt-cli/crates/gpt/src/lib.rs)
  - parses flags into stable protocol commands
  - sends daemon requests over local IPC
  - formats JSON and terminal streaming output
  - bounds CLI-side waits so user-facing commands do not hang forever
- Daemon transport shell: [`crates/gptd/src/service.rs`](/C:/Users/user/Desktop/gpt-cli/crates/gptd/src/service.rs)
  - accepts IPC requests
  - maps transport frames to `CoreService`
  - emits stream events and error responses
- Provider gateway: [`crates/tablinq-gateway/src/lib.rs`](/C:/Users/user/Desktop/gpt-cli/crates/tablinq-gateway/src/lib.rs)
  - exposes HTTP endpoints
  - validates provider-facing requests
  - queues browser-backed upstream work safely
  - records provider request summaries and recovery outcomes
  - calls `CoreService` directly instead of reusing CLI presentation code

## Internal protocols

- Stable shared command, event, status, and result types live in [`crates/shared-protocol/src/lib.rs`](/C:/Users/user/Desktop/gpt-cli/crates/shared-protocol/src/lib.rs).
- Daemon IPC uses `DaemonRequest`, `DaemonResponse`, `DaemonFrame`, and `EventPayload`.
- Browser-adapter transport remains an internal daemon detail in [`crates/gptd/src/browser.rs`](/C:/Users/user/Desktop/gpt-cli/crates/gptd/src/browser.rs).

## State layout

State-root helpers live in [`crates/shared-protocol/src/lib.rs`](/C:/Users/user/Desktop/gpt-cli/crates/shared-protocol/src/lib.rs) via `resolve_state_root` and `build_state_paths`.

Persisted directories:

- `config/`
- `logs/`
- `db/`
- `browser-profile/`
- `downloads/`
- `crash-bundles/`
- `gateway-uploads/`

SQLite entities currently live in [`crates/gptd/src/db.rs`](/C:/Users/user/Desktop/gpt-cli/crates/gptd/src/db.rs):

- `sessions`
- `jobs`
- `job_details`
- `job_artifacts`
- `artifacts`
- `conversations`
- `failure_bundles`
- `request_history`

## Architectural implication

- New provider-facing behavior should usually land in `CoreService` or the gateway, not in CLI presentation code.
- Browser, SQLite, interruption recovery, and failure-bundle behavior should stay centralized in the daemon core.
- Provider policy belongs in the gateway layer.
- Browser-driving behavior belongs in `BrowserManager` and the TypeScript adapter.
