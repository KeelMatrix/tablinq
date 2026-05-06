# Development

This guide is for contributors working from the repo checkout.

## Local setup

1. Install Rust with `cargo` on `PATH`.
2. Install Node.js 20 or newer.
3. Install `pnpm`.
4. Run `pnpm install` at the repo root.

When validating checkout changes, prefer `cargo run -p gpt -- ...`.

Plain `gpt` resolves to the installed binary on your `PATH`, usually under `%LOCALAPPDATA%\Tablinq\install\bin`, and can lag the repo checkout until `scripts/install.ps1` finishes and you start a new shell.

## Current product model

Tablinq is attach-first.

That means the normal development posture is:

- an attach target browser profile you can inspect yourself
- the dedicated Chrome attach profile under the state root when using current Chrome
- the started Edge profile when using Edge attach-first
- manual sign-in to ChatGPT
- Tablinq extension loaded in that profile
- local debug endpoint enabled
- Tablinq attaches to the existing browser session

Do not design new work around the old Playwright-owned managed-login flow. That default was intentionally abandoned because Cloudflare and anti-bot protection at `chatgpt.com` made automation-owned startup and login the wrong operational model. Playwright is still part of the codebase, but now as an attach-and-assist runtime.

For the broader system view, see [architecture.md](architecture.md).

## Standard validation

Choose the smallest loop that proves the change. Common focused loops:

- CLI-local Rust: `cargo test -p gpt`
- Daemon-local Rust: `cargo test -p gptd`
- Gateway-local Rust: `cargo test -p tablinq-gateway`
- TypeScript core/protocol/probe checks: `pnpm run test:ts:fast`
- Browser-adapter shards:
  - `pnpm run test:browser-adapter:foundation`
  - `pnpm run test:browser-adapter:ask`
  - `pnpm run test:browser-adapter:research`
  - `pnpm run test:browser-adapter:recovery`

Broader repo checks:

- `cargo test`
- `pnpm lint`
- `pnpm build`
- `pnpm test`
- `pnpm run ci:local`
- `pnpm run ci:local:all`

Formatting:

- `cargo fmt --all --check`
- `pnpm format`

## Common local commands

- `cargo run -p gpt -- daemon start`
- `cargo run -p gpt -- daemon status`
- `cargo run -p gpt -- gateway start`
- `cargo run -p gpt -- gateway status`
- `cargo run -p gpt -- ask "Name three edge cases for resumable downloads."`
- `cargo run -p gpt -- ask --mode search "Which Rust release first stabilized let-chains?"`
- `cargo run -p gpt -- ask --model thinking --effort extended "Choose SQLite or JSONL for a local job ledger."`
- `cargo run -p gpt -- ask --mode search --model thinking --effort standard "Find current Windows code-signing costs."`
- `cargo run -p gpt -- ask --mode search --conversation conv-123 "Refresh the shipping checklist against current notarization rules."`
- `cargo run -p gpt -- ask --file crash.log --file repro.md "Find the first failing step and likely owner."`
- `cargo run -p gpt -- ask --conversation conv-123 "Turn the last answer into a migration checklist."`
- `cargo run -p gpt -- ask --new-conversation "Sketch a smoke test for a stuck progress bar."`
- `cargo run -p gpt -- research "Find launch risks for a Windows tray app."`
- `cargo run -p gpt -- research --file notes.txt --output-dir reports "Write a two-page vendor-risk brief."`
- `cargo run -p gpt -- research --file roadmap.md --file support.csv --output-dir reports "Prioritize churn risks for next quarter."`
- `cargo run -p gpt -- jobs list`
- `cargo run -p gpt -- jobs show job-123`
- `cargo run -p gpt -- jobs cancel job-123`
- `cargo run -p gpt -- sessions list`
- `cargo run -p gpt -- conversations list`
- `cargo run -p gpt -- conversations list --json`
- `cargo run -p gpt -- conversations switch conv-123`
- `cargo run -p gpt -- conversations new`
- `cargo run -p gpt -- conversations new --json`
- `cargo run -p gpt -- artifacts list`
- `cargo run -p gpt -- artifacts pull last --to .`
- `cargo run -p gpt -- clear --all --force`
- `cargo run -p gpt -- doctor`
- `cargo run -p gpt -- capabilities`
- `cargo run -p gpt -- config show`
- `cargo run -p gpt -- browser start-debug`
- `cargo run -p gpt -- browser attach`
- `cargo run -p gpt -- browser connection-info`
- `cargo run -p gpt -- browser open`
- `cargo run -p gpt -- browser detach`
- `cargo run -p gpt -- browser profile-info`

## Local cleanup

- `cargo clean`
  Removes the Rust `target/` directory, including debug and release binaries, incremental compiler caches, PDBs, and package-smoke build outputs.
- Use it when the repo checkout grows large from repeated `cargo run`, `cargo test`, or packaging work.
- The next Rust build will take longer because Cargo must rebuild artifacts from scratch.

## Local browser workflow for real-site checks

Use this only when the fake harness is not enough:

1. Start the daemon and gateway.
2. If you are using Edge attach-first, load the unpacked extension into the Edge profile you will attach to.
3. Fully close the browser.
4. Run `cargo run -p gpt -- browser start-debug`.
5. If you are using Chrome attach-first, load the unpacked extension into the dedicated Chrome attach profile created under the Tablinq state root.
6. Sign in manually at `https://chatgpt.com/` in that profile.
7. Run `cargo run -p gpt -- browser attach`.
8. Use `cargo run -p gpt -- status`, `cargo run -p gpt -- doctor`, and `cargo run -p gpt -- capabilities` before testing ask or research flows.

Managed fallback exists only for debugging, CI, harness isolation, and machines that cannot expose a usable local debug endpoint.

## Windows-first helper scripts

- `scripts/bootstrap.ps1`
  - verifies the local toolchain and fixes the current PowerShell session `PATH` when needed
- `scripts/test.ps1`
  - runs Rust and TypeScript validation together
- `scripts/package.ps1`
  - builds release Rust binaries, emits browser adapter and extension JavaScript, and assembles `dist/package/tablinq`
- `scripts/install.ps1`
  - the Windows all-in-one entry point: installs dependencies, packages the repo, replaces `%LOCALAPPDATA%\Tablinq\install`, registers the native host, removes scheduled-task and Run-key autostart persistence, starts daemon and gateway by default, optionally enables autostart with `-EnableAutoStart`, and prints the installed CLI build, current-shell `gpt` resolution, repo HEAD check, and remaining manual browser-extension steps

## CI expectations

GitHub Actions currently uses:

- `.github/workflows/ci.yml`
  - Rust formatting, clippy, and locked workspace tests
  - TypeScript core formatting, type-check, build, and fast schema/probe/content-script tests
  - browser-adapter test shards for foundation, ask, research, and recovery flows
- `.github/workflows/package-smoke.yml`
  - Windows packaged install smoke coverage
  - package artifact upload on non-PR runs

On Windows, `scripts/ci-local.ps1 -Job quick` runs the default fast mirror, `-Job all` runs the non-package CI mirror, and `-Job package-smoke` runs the separate package-smoke path.

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
- attach-mode and managed-fallback browser behavior
- streaming, stall handling, interruption recovery, and reload behavior
- file upload and artifact flows
- research scenarios

Prefer the fake harness first. Use [live-smoke-checklist.md](live-smoke-checklist.md) only when you need an explicit real-site confirmation.

## Code hotspots

- `crates/gpt/src/lib.rs`
  - top-level CLI wiring into focused presentation modules such as `cli.rs`, `daemon.rs`, `browser.rs`, `status.rs`, and `terminal.rs`
- `crates/gptd/src/core/`
  - durable execution logic split across orchestration, lifecycle support, and persistence-facing helpers
- `crates/gptd/src/browser/`
  - browser-adapter supervision, request transport, cancellation, and recovery handoff
- `crates/tablinq-gateway/src/lib.rs`
  - gateway entry wiring into `server.rs`, `connection.rs`, `routes.rs`, `handlers/*`, `chat/*`, `scheduler.rs`, `uploads.rs`, `capabilities.rs`, and `process_control.rs`
- `packages/browser-adapter/src/browserAdapter.ts`
  - high-level attach runtime orchestration over focused modules under `browserAdapter/*`, `execution/*`, `pageRuntime/*`, `contracts.ts`, `timing.ts`, `assets.ts`, and `urls.ts`
- `packages/browser-adapter/src/pageAdapterRuntime/*`
  - page-level DOM helpers and UI-specific logic; `pageAdapter.runtime.js` is the bundled runtime entry
- `packages/extension/src/probe.ts`
  - extension-facing capability probes and alias discovery, with DOM/runtime helpers in `probe.runtime.js`

## Windows Toolchain Failures

See [troubleshooting](troubleshooting.md#antivirus-blocking-local-tools-or-browser-startup) before debugging Windows `Access is denied`, blocked-process, or quarantine failures.
