# Developer Workflow

## Local setup

- Install Rust and ensure `cargo` is available on `PATH`.
- Install Node.js 20 or newer.
- Install `pnpm`.
- Run `pnpm install` at the repo root.

## Primary commands

- `cargo test`
- `pnpm test`
- `pnpm lint`
- `pnpm build`

## Windows-first helper scripts

- `scripts/bootstrap.ps1`: checks the local toolchain and adds the user Rust install to `PATH` for the current PowerShell session when needed.
- `scripts/test.ps1`: runs `cargo test` and `pnpm test` in one step.

## Current testing strategy

- Rust tests cover protocol serialization, tagged payloads, validation, and CLI golden parsing stubs.
- TypeScript tests validate the JSON schemas and snapshot the selector-pack parser output.
- `packages/fake-chatgpt` is the CI-safe target for later Playwright and extension tests.

## Next implementation boundary

Part 2 should build the real control plane behind the already-frozen contracts:

- single-user daemon process model
- local IPC
- config and state directories
- SQLite persistence
- daemon lifecycle commands in the CLI
