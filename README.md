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

- `crates/gpt`: CLI entry point, daemon bootstrap, and local control commands.
- `crates/gptd`: daemon service, local IPC, config/state bootstrap, lock handling, and SQLite persistence.
- `crates/shared-protocol`: stable Rust protocol types and validation.
- `packages/browser-adapter`: schema loader, selector-pack parser, and TypeScript contract tests.
- `packages/extension`: MV3 extension scaffold.
- `packages/fake-chatgpt`: local HTML harness for future browser and extension tests.
- `schemas/v1`: frozen JSON protocol contracts.
- `selector-packs/v1`: versioned selector data.
- `docs`: architecture and developer workflow notes.
- `scripts`: bootstrap and full-suite helpers.

## Quickstart

1. Install Rust with `cargo` on `PATH`, Node.js 20+, and `pnpm`.
2. Run `pnpm install`.
3. Run `cargo test`.
4. Run `pnpm test`.
5. Start the local control plane with `cargo run -p gpt -- daemon start`.
6. Inspect it with `cargo run -p gpt -- daemon status` or `cargo run -p gpt -- doctor`.
7. Optionally start the fake harness with `pnpm --filter @tablinq/fake-chatgpt start`.

## Part 2 Scope

This repository now includes the first working control plane. It still does not launch a real browser adapter, but the daemon and CLI lifecycle are real. The stable boundaries in this part are:

- internal enums for `mode`, `model_class`, and `effort`
- JSON schema definitions for commands, events, streamed chunks, capability reports, job states, artifacts, failure bundles, and health checks
- a selector-pack file format kept outside compiled code
- a single-user daemon with local socket IPC, lock recovery, config bootstrap, structured log file output, and SQLite migrations
- CLI commands for `daemon start|status|stop`, `doctor`, `config show`, `sessions list`, and `jobs list`

## Implemented Commands

- `gpt daemon start`
- `gpt daemon status`
- `gpt daemon stop`
- `gpt doctor`
- `gpt config show`
- `gpt sessions list`
- `gpt jobs list`

`ask`, `research`, browser control, and artifact commands remain intentionally deferred to later parts.

More detail lives in [docs/architecture.md](/C:/Users/user/Desktop/gpt-cli/docs/architecture.md) and [docs/development.md](/C:/Users/user/Desktop/gpt-cli/docs/development.md).
