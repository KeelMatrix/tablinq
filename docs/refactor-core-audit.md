# Core Audit

This note captures the current separation of concerns now that Tablinq has a browser-backed provider gateway, durable request lifecycle state, and interruption recovery.

## Current separation

- Core execution engine: [`crates/gptd/src/core/mod.rs`](../crates/gptd/src/core/mod.rs)
  - owns ask and research execution
  - owns request lifecycle transitions and recovery markers
  - owns browser lifecycle calls
  - owns job, session, conversation, artifact, request-summary, and failure-bundle persistence
  - delegates shared lifecycle helpers to [`crates/gptd/src/core/support.rs`](../crates/gptd/src/core/support.rs) and control-plane operations to [`crates/gptd/src/core/operations.rs`](../crates/gptd/src/core/operations.rs)
  - calls the Playwright and extension runtime only through [`BrowserManager`](../crates/gptd/src/browser/mod.rs)
- Admin CLI: [`crates/gpt/src/lib.rs`](../crates/gpt/src/lib.rs)
  - parses flags into stable protocol commands
  - sends daemon requests over local IPC
  - routes feature-specific presentation through modules such as [`crates/gpt/src/cli.rs`](../crates/gpt/src/cli.rs), [`crates/gpt/src/daemon.rs`](../crates/gpt/src/daemon.rs), [`crates/gpt/src/browser.rs`](../crates/gpt/src/browser.rs), and [`crates/gpt/src/terminal.rs`](../crates/gpt/src/terminal.rs)
  - bounds CLI-side waits so user-facing commands do not hang forever
- Daemon transport shell: [`crates/gptd/src/service.rs`](../crates/gptd/src/service.rs)
  - accepts IPC requests
  - maps transport frames to `CoreService`
  - emits stream events and error responses
- Provider gateway: [`crates/tablinq-gateway/src/lib.rs`](../crates/tablinq-gateway/src/lib.rs)
  - exposes HTTP startup through [`crates/tablinq-gateway/src/server.rs`](../crates/tablinq-gateway/src/server.rs), [`crates/tablinq-gateway/src/connection.rs`](../crates/tablinq-gateway/src/connection.rs), and [`crates/tablinq-gateway/src/routes.rs`](../crates/tablinq-gateway/src/routes.rs)
  - delegates endpoint handling to [`crates/tablinq-gateway/src/handlers`](../crates/tablinq-gateway/src/handlers)
  - delegates provider request mapping and streaming to [`crates/tablinq-gateway/src/chat`](../crates/tablinq-gateway/src/chat)
  - delegates queueing to [`crates/tablinq-gateway/src/scheduler.rs`](../crates/tablinq-gateway/src/scheduler.rs)
  - delegates staged upload handling to [`crates/tablinq-gateway/src/uploads.rs`](../crates/tablinq-gateway/src/uploads.rs)
  - records provider request summaries and recovery outcomes
  - calls `CoreService` directly instead of reusing CLI presentation code

## Internal protocols

- Stable shared command, event, status, and result types live in [`crates/shared-protocol/src/lib.rs`](../crates/shared-protocol/src/lib.rs).
- Daemon IPC uses `DaemonRequest`, `DaemonResponse`, `DaemonFrame`, and `EventPayload`.
- Browser-adapter transport remains an internal daemon detail in [`crates/gptd/src/browser/mod.rs`](../crates/gptd/src/browser/mod.rs) and its supporting protocol/runtime modules.

## State layout

State-root helpers live in [`crates/shared-protocol/src/lib.rs`](../crates/shared-protocol/src/lib.rs) via `resolve_state_root` and `build_state_paths`.

Persisted directories:

- `config/`
- `logs/`
- `db/`
- `browser-profile/`
- `chrome-attach-profile/`
- `downloads/`
- `crash-bundles/`
- `gateway-uploads/`

SQLite entities currently live in [`crates/gptd/src/db.rs`](../crates/gptd/src/db.rs):

- `schema_migrations`
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
