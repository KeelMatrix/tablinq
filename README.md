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
5. Optionally start the fake harness with `pnpm --filter @tablinq/fake-chatgpt start`.
6. Start the local control plane with `cargo run -p gpt -- daemon start`.
7. Open a managed browser with `cargo run -p gpt -- browser open`.
8. Inspect it with `cargo run -p gpt -- browser profile-info` or `cargo run -p gpt -- doctor`.

## Part 4 Scope

This repository now includes a semantic probe layer on top of the managed browser runtime. The stable boundaries in this part are:

- internal enums for `mode`, `model_class`, and `effort`
- JSON schema definitions for commands, events, streamed chunks, capability reports, job states, artifacts, failure bundles, and health checks
- a selector-pack file format kept outside compiled code
- a single-user daemon with local socket IPC, lock recovery, config bootstrap, structured log file output, and SQLite migrations
- a Playwright-backed browser adapter with a persistent dedicated profile, managed tab recovery, screenshots, and extension-driven capability snapshots
- an MV3 extension service worker and content script with native-messaging and localhost WebSocket bridge modes
- ARIA-aware capability probes with strategy scoring, DOM neighborhood snapshots, and last-known-good strategy reuse per observed UI shape
- fake harness coverage for both the classic slot-based UI and an ARIA-first fallback UI
- CLI commands for `daemon start|status|stop`, `doctor`, `capabilities`, `config show`, `sessions list`, `jobs list`, and `browser open|close|reset-profile|profile-info`

## Implemented Commands

- `gpt daemon start`
- `gpt daemon status`
- `gpt daemon stop`
- `gpt doctor`
- `gpt capabilities`
- `gpt config show`
- `gpt sessions list`
- `gpt jobs list`
- `gpt browser open`
- `gpt browser close`
- `gpt browser reset-profile`
- `gpt browser profile-info`

`ask`, `research`, uploads, downloads, and artifacts remain deferred to later parts.

More detail lives in [docs/architecture.md](/C:/Users/user/Desktop/gpt-cli/docs/architecture.md) and [docs/development.md](/C:/Users/user/Desktop/gpt-cli/docs/development.md).
