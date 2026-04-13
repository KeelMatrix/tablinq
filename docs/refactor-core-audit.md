# Core Audit

This note captures the current core separation now that the provider gateway exists.

## Current separation

- Core execution engine: [`crates/gptd/src/core.rs`](/C:/Users/user/Desktop/gpt-cli/crates/gptd/src/core.rs)
  - owns ask and research execution
  - owns browser lifecycle calls
  - owns job, session, conversation, artifact, and failure-bundle persistence
  - calls the Playwright and extension runtime only through [`BrowserManager`](/C:/Users/user/Desktop/gpt-cli/crates/gptd/src/browser.rs)
- Admin CLI: [`crates/gpt/src/lib.rs`](/C:/Users/user/Desktop/gpt-cli/crates/gpt/src/lib.rs)
  - parses flags into stable protocol commands
  - sends daemon requests over local IPC
  - formats JSON and terminal streaming output
- Daemon transport shell: [`crates/gptd/src/service.rs`](/C:/Users/user/Desktop/gpt-cli/crates/gptd/src/service.rs)
  - accepts IPC requests
  - maps transport frames to `CoreService`
  - emits stream events and error responses
- Provider gateway: [`crates/tablinq-gateway/src/lib.rs`](/C:/Users/user/Desktop/gpt-cli/crates/tablinq-gateway/src/lib.rs)
  - exposes HTTP endpoints
  - validates provider-facing requests
  - queues upstream work safely
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

## Architectural implication

- New provider-facing features should usually land in `CoreService` or the gateway, not in CLI presentation code.
- Browser, SQLite, and failure-bundle behavior should stay centralized in the daemon core.
- Provider-specific policy belongs in the gateway layer, while browser-driving behavior belongs in `BrowserManager` and the TypeScript adapter.
