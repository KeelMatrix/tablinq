# Development

This guide is for contributors working from the repo checkout.

## Local setup

1. Install Rust with `cargo` on `PATH`.
2. Install Node.js 20 or newer.
3. Install `pnpm`.
4. Run `pnpm install` at the repo root.

## Fast mental model

- Rust owns the CLI, daemon, gateway, config, persistence, logging, and packaging flow.
- TypeScript owns the browser adapter, extension bridge, and fake harness.
- The browser is still the execution engine.
- `packages/fake-chatgpt` is the CI-safe browser target for most automated browser tests.

For the deeper system view, see [architecture.md](architecture.md).

## Standard validation

Run these before opening a PR:

- `cargo test`
- `pnpm lint`
- `pnpm build`
- `pnpm test`

Formatting:

- `cargo fmt --all --check`
- `pnpm format`

## Common local commands

- `cargo run -p gpt -- daemon start`
- `cargo run -p gpt -- daemon status`
- `cargo run -p gpt -- gateway start`
- `cargo run -p gpt -- gateway status`
- `cargo run -p gpt -- ask "hello world"`
- `cargo run -p gpt -- ask --model instant --mode search "latest release notes"`
- `cargo run -p gpt -- ask --model thinking --effort extended "compare these ideas"`
- `cargo run -p gpt -- research "investigate launch blockers"`
- `cargo run -p gpt -- jobs list`
- `cargo run -p gpt -- jobs show job-123`
- `cargo run -p gpt -- jobs cancel job-123`
- `cargo run -p gpt -- sessions list`
- `cargo run -p gpt -- artifacts list`
- `cargo run -p gpt -- artifacts pull last`
- `cargo run -p gpt -- doctor`
- `cargo run -p gpt -- capabilities`
- `cargo run -p gpt -- config show`
- `cargo run -p gpt -- browser open`
- `cargo run -p gpt -- browser logout`
- `cargo run -p gpt -- browser switch-account`
- `cargo run -p gpt -- browser reset-profile`
- `cargo run -p gpt -- browser profile-info`

## Windows-first helper scripts

- `scripts/bootstrap.ps1`
  - verifies the local toolchain and fixes the current PowerShell session `PATH` when needed
- `scripts/test.ps1`
  - runs Rust and TypeScript validation together
- `scripts/package.ps1`
  - builds release Rust binaries, emits browser adapter and extension JavaScript, and assembles `dist/package/tablinq`
- `scripts/install.ps1`
  - installs the package under `%LOCALAPPDATA%\Tablinq\install`, initializes `%LOCALAPPDATA%\Tablinq`, registers the native host, and writes startup helpers

## CI expectations

GitHub Actions currently uses:

- `.github/workflows/ci.yml`
  - Rust formatting, clippy, and locked workspace tests
  - TypeScript formatting, type-check, build, and tests
- `.github/workflows/package-smoke.yml`
  - Windows packaged install smoke coverage
  - package artifact upload on non-PR runs

Dependabot PRs still run the main Rust and TypeScript checks. The expensive Windows package smoke run is skipped for Dependabot PRs.

## Testing strategy

Rust tests cover:

- protocol serialization
- CLI parsing and presentation
- daemon startup and IPC
- config bootstrap and migrations
- SQLite persistence
- browser health and orchestration
- gateway request handling, queueing, and failure mapping
- packaging and installed-layout smoke coverage on Windows

TypeScript tests cover:

- JSON schemas
- extension probes
- selector-pack loading
- fake-harness browser flows
- streaming, stall handling, interruption recovery, and reload behavior
- file upload and artifact flows
- research scenarios

## Code hotspots

- `crates/gpt/src/lib.rs`
  - CLI command surface and terminal presentation
- `crates/gptd/src/core.rs`
  - durable execution logic, state transitions, and persistence boundaries
- `crates/gptd/src/browser.rs`
  - browser-adapter supervision and recovery handoff
- `crates/tablinq-gateway/src/lib.rs`
  - HTTP provider surface, queueing, and request summaries
- `packages/browser-adapter/src/browserAdapter.ts`
  - Playwright control loop and reconciliation behavior
- `packages/browser-adapter/src/pageAdapter.runtime.js`
  - page-level DOM helpers and UI-specific logic
- `packages/extension/src/probe.ts`
  - capability probes and alias discovery

## When to use the live site

Do not use the live site for routine development validation. Prefer the fake harness first. Use [live-smoke-checklist.md](live-smoke-checklist.md) only when you need an explicit real-site confirmation.
