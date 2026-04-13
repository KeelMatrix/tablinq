# Development

## Local setup

1. Install Rust with `cargo` on `PATH`.
2. Install Node.js 20 or newer.
3. Install `pnpm`.
4. Run `pnpm install` at the repo root.

## Primary validation commands

- `cargo test`
- `pnpm lint`
- `pnpm build`
- `pnpm test`

## Common runtime commands

- `cargo run -p gpt -- daemon start`
- `cargo run -p gpt -- daemon status`
- `cargo run -p gpt -- gateway start`
- `cargo run -p gpt -- gateway status`
- `cargo run -p gpt -- ask "hello world"`
- `cargo run -p gpt -- ask --model instant --mode search "latest release notes"`
- `cargo run -p gpt -- ask --model thinking --effort extended "compare these ideas"`
- `cargo run -p gpt -- research "investigate launch blockers"`
- `cargo run -p gpt -- artifacts list`
- `cargo run -p gpt -- artifacts pull last`
- `cargo run -p gpt -- jobs list`
- `cargo run -p gpt -- jobs show job-123`
- `cargo run -p gpt -- jobs cancel job-123`
- `cargo run -p gpt -- doctor`
- `cargo run -p gpt -- capabilities`
- `cargo run -p gpt -- config show`
- `cargo run -p gpt -- browser open`
- `cargo run -p gpt -- browser logout`
- `cargo run -p gpt -- browser switch-account`
- `cargo run -p gpt -- browser reset-profile`
- `cargo run -p gpt -- browser profile-info`

## Windows-first helper scripts

- `scripts/bootstrap.ps1`: checks the local toolchain and adds the user Rust install to `PATH` for the current PowerShell session when needed.
- `scripts/test.ps1`: runs `cargo test` and `pnpm test`.
- `scripts/package.ps1`: builds release Rust binaries, emits browser adapter and extension JavaScript, and assembles `dist/package/tablinq`.
- `scripts/install.ps1`: installs the packaged layout to `%LOCALAPPDATA%\Tablinq\install`, initializes `%LOCALAPPDATA%\Tablinq`, registers the native messaging host, and writes startup helper scripts.

## CI

GitHub Actions uses two workflows:

- `.github/workflows/ci.yml`
  - Rust formatting, clippy, and workspace tests
  - TypeScript formatting, type-check, build, and tests
- `.github/workflows/package-smoke.yml`
  - Windows package smoke build through `scripts/package.ps1`
  - package artifact upload for inspection

Match those surfaces before opening a PR.

## Current testing strategy

- Rust tests cover protocol serialization, daemon startup, IPC connectivity, config bootstrap, SQLite migrations, browser health, CLI parsing, job persistence, artifacts, research, and failure-bundle behavior.
- TypeScript tests cover JSON schemas, extension probes, selector-pack loading, fake-harness browser flows, streaming, reload recovery, upload/download handling, and research scenarios.
- `packages/fake-chatgpt` is the CI-safe browser target. Live-site verification is manual and documented separately.

## Code ownership hotspots

- `crates/gpt/src/lib.rs`: CLI command surface and presentation
- `crates/gptd/src/core.rs`: durable execution logic and state transitions
- `crates/gptd/src/browser.rs`: browser-adapter supervision
- `crates/tablinq-gateway/src/lib.rs`: HTTP provider surface
- `packages/browser-adapter/src/browserAdapter.ts`: Playwright flow
- `packages/browser-adapter/src/pageAdapter.runtime.js`: page-level DOM helpers
- `packages/extension/src/probe.ts`: capability probes and alias discovery
