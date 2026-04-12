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

- `crates/gpt`: CLI surface and argument parsing stubs.
- `crates/gptd`: daemon scaffold.
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
5. Optionally start the fake harness with `pnpm --filter @tablinq/fake-chatgpt start`.

## Part 1 Scope

This repository currently freezes the control protocol and monorepo layout. It does not yet launch a real browser or daemon process tree. The stable boundaries in this part are:

- internal enums for `mode`, `model_class`, and `effort`
- JSON schema definitions for commands, events, streamed chunks, capability reports, job states, artifacts, failure bundles, and health checks
- a selector-pack file format kept outside compiled code
- CLI parsing stubs and protocol validation tests

More detail lives in [docs/architecture.md](/C:/Users/user/Desktop/gpt-cli/docs/architecture.md) and [docs/development.md](/C:/Users/user/Desktop/gpt-cli/docs/development.md).
