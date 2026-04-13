# Tablinq

Tablinq is a local-first CLI and daemon that control a dedicated, logged-in ChatGPT browser session. The browser is the execution engine. Rust owns the control plane and persistence. TypeScript owns the browser adapter, the extension bridge, and the fake UI harness used for local tests.

## Architecture

```mermaid
flowchart LR
    CLI["`gpt` CLI (Rust)"] --> IPC["Local IPC<br/>named pipe / unix socket"]
    IPC --> Daemon["`gptd` daemon (Rust)"]
    Daemon --> DB["SQLite + file store"]
    Daemon --> Adapter["Browser adapter (TypeScript + Playwright)"]
    Adapter --> Browser["Managed Edge / Chromium profile"]
    Browser --> Extension["MV3 extension"]
    Extension --> Page["ChatGPT page or fake-chatgpt harness"]
    Daemon --> Schemas["Versioned JSON schemas"]
    Extension --> Selectors["Selector packs as external data"]
```

## Workspace Layout

- `crates/gpt`: CLI entry point, daemon bootstrap, local control commands, and browser lifecycle commands.
- `crates/gptd`: daemon service, local IPC, config/state bootstrap, lock handling, SQLite persistence, and browser adapter orchestration.
- `crates/shared-protocol`: stable Rust protocol types and validation.
- `packages/browser-adapter`: Playwright-backed browser runtime, service entrypoint, selector-pack helpers, and browser tests.
- `packages/extension`: MV3 extension bridge, capability probes, and extension unit tests.
- `packages/fake-chatgpt`: local HTML harness with multiple UI variants for browser and extension tests.
- `schemas/v1`: frozen JSON protocol contracts.
- `selector-packs/v1`: versioned selector data.
- `docs`: architecture and developer workflow notes.
- `scripts`: bootstrap and full-suite helpers.

## Quickstart

1. Install Rust with `cargo` on `PATH`, Node.js 20+, and `pnpm`.
2. Run `pnpm install`.
3. Run `cargo test`.
4. Run `pnpm test`.
5. Build installable artifacts with `scripts/package.ps1` on Windows or `./scripts/package.sh` on Unix-like systems.
6. Install locally with `scripts/install.ps1` on Windows or `./scripts/install.sh` on Unix-like systems.
7. Optionally start the fake harness with `pnpm --filter @tablinq/fake-chatgpt start`.
8. Start the local control plane with `cargo run -p gpt -- daemon start`.
9. Start the provider gateway with `cargo run -p gpt -- gateway start`.
10. Open a managed browser with `cargo run -p gpt -- browser open`.
11. Inspect it with `cargo run -p gpt -- status` or `cargo run -p gpt -- doctor`.

Install, provider, and troubleshooting detail lives in [docs/install.md](/C:/Users/user/Desktop/gpt-cli/docs/install.md), [docs/provider-mode.md](/C:/Users/user/Desktop/gpt-cli/docs/provider-mode.md), and [docs/troubleshooting.md](/C:/Users/user/Desktop/gpt-cli/docs/troubleshooting.md).

## Part 11 Scope

This repository now includes the Part 11 hardening and live-site adaptation pass on top of the packaged local install flow. The stable boundaries in this part are:

- internal enums for `mode`, `model_class`, and `effort`
- JSON schema definitions for commands, events, streamed chunks, capability reports, job states, artifacts, failure bundles, and health checks
- a selector-pack file format kept outside compiled code
- a single-user daemon with local socket IPC, lock recovery, config bootstrap, structured log file output, and SQLite migrations
- a Playwright-backed browser adapter with a persistent dedicated profile, managed tab recovery, screenshots, and extension-driven capability snapshots
- an MV3 extension service worker and content script with native-messaging and localhost WebSocket bridge modes
- ARIA-aware capability probes with strategy scoring, DOM neighborhood snapshots, and last-known-good strategy reuse per observed UI shape
- fake harness coverage for both the classic slot-based UI and an ARIA-first fallback UI
- a streamed `ask` flow from CLI -> daemon -> browser adapter -> managed page, with durable job and conversation persistence
- runtime model, effort, and mode selection driven by discovered capability aliases instead of hardcoded UI labels
- persisted per-workspace capability alias and strategy cache under the managed browser profile
- persisted strategy success/failure telemetry per capability and strategy
- clear capability-missing failures when a requested model, effort, or mode is unavailable in the current UI
- fake-harness coverage for successful streaming, timeout handling, tab reload recovery, renamed controls, and missing controls during an active ask
- Playwright-backed file attachment with page acknowledgement checks before submit
- managed artifact download capture, checksum recording, and durable SQLite-backed artifact metadata
- a separate research execution path with durable `queued`, `starting`, `running`, `waiting_for_sources`, `completed`, and `failed` job states
- persisted job detail lookup and cancellation requests through `jobs show` and `jobs cancel`
- local failure bundles with screenshot, DOM snapshot, ARIA snapshot, active URL, strategy attempts, capability matrix, and logs linkage
- selector-pack loading from external versioned JSON instead of adapter-local metadata only
- daemon-side recovery guidance for browser relaunch, tab recreation, profile access problems, auth-required cases, and safe mode
- real emitted JavaScript builds for the browser adapter and MV3 extension instead of source-only package stubs
- package/install scripts for Windows and Unix-like local installs
- a dedicated user-level native messaging host binary and registration flow for Windows
- explicit `gpt version` and `gptd version` build metadata output
- an isolated in-page runtime module for live-site composer, submit, mode/model/effort, upload acknowledgement, ask snapshot, and research snapshot actions
- live-like selector-pack and probe coverage for current ChatGPT-style `#prompt-textarea`, send/stop buttons, model/mode controls, upload entry points, and download affordances
- conservative timeout, retry, and recovery policies for navigation, probing, interaction-surface waits, upload acknowledgement, streaming completion, and managed download capture
- post-download managed-page recovery that recreates the tab when a download invalidates the active frame
- a live-like fake harness variant that exercises current ChatGPT-shaped controls while keeping CI on local fixtures only
- an optional manual live-site smoke checklist for login, ask, mode/model selection, upload, download, and research verification
- CLI commands for `ask`, `research`, `artifacts list`, `artifacts pull`, `daemon start|status|stop`, `doctor`, `capabilities`, `config show`, `sessions list`, `jobs list|show|cancel`, and `browser open|close|reset-profile|profile-info`
- packaged extension assets and stable extension identity for unpacked local loads

## Implemented Commands

- `gpt version`
- `gpt status`
- `gpt daemon start`
- `gpt daemon status`
- `gpt daemon stop`
- `gpt gateway start`
- `gpt gateway status`
- `gpt gateway stop`
- `gpt ask "hello"`
- `gpt ask --model instant --mode search "hello"`
- `gpt ask --model thinking --effort extended "hello"`
- `gpt ask --file notes.txt --file data.csv "summarize uploads"`
- `gpt research "investigate launch blockers" --file notes.txt`
- `gpt artifacts list`
- `gpt artifacts pull last`
- `gpt jobs show job-123`
- `gpt jobs cancel job-123`
- `gpt doctor`
- `gpt capabilities`
- `gpt config show`
- `gpt sessions list`
- `gpt jobs list`
- `gpt browser open`
- `gpt browser close`
- `gpt browser reset-profile`
- `gpt browser profile-info`
- `gptd version`

Optional real-site verification lives in [docs/live-smoke-checklist.md](/C:/Users/user/Desktop/gpt-cli/docs/live-smoke-checklist.md).

More detail lives in [docs/architecture.md](/C:/Users/user/Desktop/gpt-cli/docs/architecture.md), [docs/development.md](/C:/Users/user/Desktop/gpt-cli/docs/development.md), [docs/install.md](/C:/Users/user/Desktop/gpt-cli/docs/install.md), [docs/troubleshooting.md](/C:/Users/user/Desktop/gpt-cli/docs/troubleshooting.md), and [docs/live-smoke-checklist.md](/C:/Users/user/Desktop/gpt-cli/docs/live-smoke-checklist.md).
