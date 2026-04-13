# Core Audit

This audit freezes the current internal core before adding a provider-facing gateway.

## Current Separation

- Core browser execution engine: [`crates/gptd/src/core.rs`](/C:/Users/user/Desktop/gpt-cli/crates/gptd/src/core.rs)
  - Owns ask/research job execution, browser lifecycle calls, capability lookup, artifact lookup/pull, health snapshots, session persistence, and failure bundle capture.
  - Calls the Playwright/extension runtime only through [`BrowserManager`](/C:/Users/user/Desktop/gpt-cli/crates/gptd/src/browser.rs).
- Admin CLI: [`crates/gpt/src/lib.rs`](/C:/Users/user/Desktop/gpt-cli/crates/gpt/src/lib.rs)
  - Parses flags into stable protocol commands.
  - Sends daemon requests over local IPC.
  - Formats JSON and terminal streaming output as a presentation layer.
- Daemon transport shell: [`crates/gptd/src/service.rs`](/C:/Users/user/Desktop/gpt-cli/crates/gptd/src/service.rs)
  - Accepts IPC requests.
  - Maps transport frames to `CoreService` calls.
  - Emits stream events and error responses.
- Future provider service: not implemented yet.
  - The intended call surface is `CoreService`, not CLI parsing or stdout scraping.

## Internal Protocols

- Stable shared command/event/result types live in [`crates/shared-protocol/src/lib.rs`](/C:/Users/user/Desktop/gpt-cli/crates/shared-protocol/src/lib.rs).
- Daemon IPC currently uses:
  - `DaemonRequest`
  - `DaemonResponse`
  - `DaemonFrame`
  - `EventPayload`
- Browser adapter transport remains an internal daemon detail in [`crates/gptd/src/browser.rs`](/C:/Users/user/Desktop/gpt-cli/crates/gptd/src/browser.rs).

## Command Flow

1. `gpt` parses CLI arguments.
2. `gpt` builds protocol commands or request enums.
3. `gpt` sends them to `gptd` over local socket IPC.
4. `gptd::service` decodes the request and calls `CoreService`.
5. `CoreService` persists job/session/artifact state in SQLite and drives `BrowserManager`.
6. `BrowserManager` talks to the TypeScript browser adapter process, which in turn uses Playwright plus the extension bridge.
7. Streaming chunks and job-state events flow back up as `EventPayload`.

## State Layout

- State root resolution and directory layout live in [`crates/shared-protocol/src/lib.rs`](/C:/Users/user/Desktop/gpt-cli/crates/shared-protocol/src/lib.rs) via `resolve_state_root` and `build_state_paths`.
- Current persisted directories:
  - `config/`
  - `logs/`
  - `db/`
  - `browser-profile/`
  - `downloads/`
  - `crash-bundles/`
- Current persisted SQLite entities live in [`crates/gptd/src/db.rs`](/C:/Users/user/Desktop/gpt-cli/crates/gptd/src/db.rs):
  - `sessions`
  - `jobs`
  - `job_details`
  - `job_artifacts`
  - `artifacts`
  - `conversations`
  - `failure_bundles`

## Browser Session Ownership

- The daemon is the sole owner of the managed browser session and dedicated profile.
- The CLI never talks to the browser directly.
- The adapter process is created and supervised from `BrowserManager`.
- Cancellation and long-running research state are coordinated inside `CoreService`, not in the CLI.

## Provider-Layer Implication

- A future local provider gateway should call `CoreService` directly.
- It should not reuse terminal formatting code from `gpt`.
- It should not duplicate browser, SQLite, or failure-bundle logic already owned by `CoreService`.
