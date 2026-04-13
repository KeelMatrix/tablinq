# Tablinq

Tablinq is a local-first CLI, daemon, and HTTP gateway that drive a dedicated ChatGPT browser session. The browser is the execution engine. Rust owns the control plane, persistence, and gateway. TypeScript owns the browser adapter, extension bridge, and fake harness used for CI-safe browser tests.

## What it does

- runs a managed browser profile for ChatGPT
- exposes local CLI commands for ask, research, jobs, artifacts, browser lifecycle, and diagnostics
- exposes an OpenAI-compatible local HTTP gateway for tools such as OpenClaude and Paperclip
- persists jobs, conversations, artifacts, and failure bundles locally
- uses a fake ChatGPT harness in CI instead of relying on the live site

## Architecture

```mermaid
flowchart LR
    CLI["`gpt` CLI"] --> IPC["local IPC"]
    IPC --> Daemon["`gptd` daemon"]
    Daemon --> DB["SQLite + file state"]
    Daemon --> Adapter["browser adapter (TS + Playwright)"]
    Adapter --> Browser["managed Edge / Chromium profile"]
    Browser --> Extension["MV3 extension"]
    Extension --> Page["ChatGPT page or fake-chatgpt"]
    Gateway["HTTP gateway"] --> Daemon
    Daemon --> Schemas["schemas + selector packs"]
```

## Workspace layout

- `crates/gpt`: CLI entry point and local control commands
- `crates/gptd`: daemon service, state/config bootstrap, SQLite, browser orchestration
- `crates/shared-protocol`: stable Rust protocol types and state-path helpers
- `crates/tablinq-gateway`: local OpenAI-compatible HTTP gateway
- `crates/tablinq-host`: native messaging host for the browser extension
- `packages/browser-adapter`: Playwright-backed browser runtime
- `packages/extension`: MV3 extension bridge and capability probes
- `packages/fake-chatgpt`: fake UI harness for tests
- `schemas/v1`: frozen protocol contracts
- `selector-packs/v1`: selector data kept outside compiled code
- `docs`: install, development, provider, troubleshooting, and architecture notes

## Quickstart

1. Install Rust, Node.js 20+, and `pnpm`.
2. Run `pnpm install`.
3. Run `cargo test`.
4. Run `pnpm test`.
5. Build a local package with `scripts/package.ps1` on Windows or `./scripts/package.sh` on Unix-like systems.
6. Install it with `scripts/install.ps1` on Windows or `./scripts/install.sh` on Unix-like systems.
7. Load the unpacked extension from the installed `extension` directory.
8. Start the daemon: `gpt daemon start`
9. Start the gateway: `gpt gateway start`
10. Open the managed browser: `gpt browser open`
11. Sign in once in the managed profile.
12. Confirm health with `gpt status` or `gpt doctor`.

## Common commands

- `gpt version`
- `gpt status`
- `gpt doctor`
- `gpt capabilities`
- `gpt config show`
- `gpt daemon start|status|stop`
- `gpt gateway start|status|stop`
- `gpt ask "hello"`
- `gpt ask --model instant --mode search "latest release notes"`
- `gpt ask --model thinking --effort extended "compare these options"`
- `gpt ask --file notes.txt "summarize the attachment"`
- `gpt research "investigate launch blockers"`
- `gpt jobs list`
- `gpt jobs show job-123`
- `gpt jobs cancel job-123`
- `gpt artifacts list`
- `gpt artifacts pull last`
- `gpt browser open`
- `gpt browser close`
- `gpt browser logout`
- `gpt browser switch-account`
- `gpt browser reset-profile`
- `gpt browser profile-info`
- `gpt sessions list`

## Managed account switching

Use either of these commands when you want to sign into a different ChatGPT account in the managed profile:

- `gpt browser logout`
- `gpt browser switch-account`

Both commands clear the managed browser profile and reopen the managed ChatGPT tab so you can sign in again cleanly.

## Testing and CI

- Rust validation: `cargo test`
- TypeScript validation: `pnpm lint`, `pnpm build`, `pnpm test`
- Formatting: `cargo fmt --all --check`, `pnpm format`
- GitHub Actions:
  - `.github/workflows/ci.yml` runs Rust and TypeScript checks on pull requests and `main`
  - `.github/workflows/package-smoke.yml` builds the Windows package layout on `main` and manual runs

## Documentation

- [Install](docs/install.md)
- [Development](docs/development.md)
- [Architecture](docs/architecture.md)
- [Provider mode](docs/provider-mode.md)
- [OpenClaude](docs/openclaude.md)
- [Paperclip](docs/paperclip.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Live smoke checklist](docs/live-smoke-checklist.md)
