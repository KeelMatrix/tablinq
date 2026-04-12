# Developer Workflow

## Local setup

- Install Rust and ensure `cargo` is available on `PATH`.
- Install Node.js 20 or newer.
- Install `pnpm`.
- Run `pnpm install` at the repo root.

## Primary commands

- `cargo test`
- `cargo run -p gpt -- daemon start`
- `cargo run -p gpt -- daemon status`
- `cargo run -p gpt -- doctor`
- `cargo run -p gpt -- config show`
- `pnpm test`
- `pnpm lint`
- `pnpm build`

## Windows-first helper scripts

- `scripts/bootstrap.ps1`: checks the local toolchain and adds the user Rust install to `PATH` for the current PowerShell session when needed.
- `scripts/test.ps1`: runs `cargo test` and `pnpm test` in one step.

## Current testing strategy

- Rust tests cover protocol serialization, daemon boot, IPC connectivity, config bootstrap, SQLite migrations, job persistence across restart, stale-lock recovery, and CLI parsing.
- TypeScript tests validate the JSON schemas and snapshot the selector-pack parser output.
- `packages/fake-chatgpt` is the CI-safe target for later Playwright and extension tests.

## Next implementation boundary

Part 3 should build the browser runtime behind the now-working control plane:

- Playwright-backed browser launch and reconnect
- persistent dedicated profile ownership
- managed browser open/close/reset commands
- daemon <-> adapter health messaging
