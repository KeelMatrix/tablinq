# Development

This guide is for contributors working from the repo checkout.

## Local setup

1. Install Rust with `cargo` on `PATH`.
2. Install Node.js 20 or newer.
3. Install `pnpm`.
4. Run `pnpm install` at the repo root.

## Current product model

Tablinq is attach-first.

That means the normal development posture is:

- normal Chrome or Edge profile
- manual sign-in to ChatGPT
- Tablinq extension loaded in that profile
- local debug endpoint enabled
- Tablinq attaches to the existing browser session

Do not design new work around the old Playwright-owned managed-login flow. That default was intentionally abandoned because Cloudflare and anti-bot protection at `chatgpt.com` made automation-owned startup and login the wrong operational model. Playwright is still part of the codebase, but now as an attach-and-assist runtime.

For the broader system view, see [architecture.md](architecture.md).

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
- `cargo run -p gpt -- ask --mode search "What is the latest stable Rust release?"`
- `cargo run -p gpt -- ask --model thinking --effort extended "compare these ideas"`
- `cargo run -p gpt -- research "investigate launch blockers"`
- `cargo run -p gpt -- jobs list`
- `cargo run -p gpt -- jobs show job-123`
- `cargo run -p gpt -- jobs cancel job-123`
- `cargo run -p gpt -- sessions list`
- `cargo run -p gpt -- conversations list`
- `cargo run -p gpt -- conversations switch conv-123`
- `cargo run -p gpt -- conversations new`
- `cargo run -p gpt -- artifacts list`
- `cargo run -p gpt -- artifacts pull last --to .`
- `cargo run -p gpt -- doctor`
- `cargo run -p gpt -- capabilities`
- `cargo run -p gpt -- config show`
- `cargo run -p gpt -- browser start-debug`
- `cargo run -p gpt -- browser attach`
- `cargo run -p gpt -- browser connection-info`
- `cargo run -p gpt -- browser open`
- `cargo run -p gpt -- browser close`
- `cargo run -p gpt -- browser logout`
- `cargo run -p gpt -- browser switch-account`
- `cargo run -p gpt -- browser reset-profile`
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
  - the Windows all-in-one entry point: installs dependencies, packages the repo, replaces `%LOCALAPPDATA%\Tablinq\install`, registers the native host, removes legacy autostart persistence, starts daemon and gateway by default, optionally enables autostart with `-EnableAutoStart`, and prints the remaining manual browser-extension steps

## CI expectations

GitHub Actions currently uses:

- `.github/workflows/ci.yml`
  - Rust formatting, clippy, and locked workspace tests
  - TypeScript formatting, type-check, build, and tests
- `.github/workflows/package-smoke.yml`
  - Windows packaged install smoke coverage
  - package artifact upload on non-PR runs

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
  - HTTP entry wiring into `chat.rs`, `scheduler.rs`, `uploads.rs`, `capabilities.rs`, and `process_control.rs`
- `packages/browser-adapter/src/browserAdapter.ts`
  - high-level attach runtime orchestration over focused modules such as `contracts.ts`, `timing.ts`, `assets.ts`, `pageRuntime.ts`, and `urls.ts`
- `packages/browser-adapter/src/pageAdapter.runtime.js`
  - page-level DOM helpers and UI-specific logic
- `packages/extension/src/probe.ts`
  - extension-facing capability probes and alias discovery, with DOM/runtime helpers in `probe.runtime.js`

## Antivirus warning on Windows

If packaging, tests, or browser startup fail with `Access is denied`, check antivirus notifications before debugging the repo. On some Windows machines, local toolchain executables such as `node.exe`, `pwsh.exe`, `chrome.exe`, `cmd.exe`, or `esbuild.exe` may need explicit antivirus exceptions.

Common exception candidates:

- `<repo-root>`
- `%LOCALAPPDATA%\Tablinq`
- `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup`
- `%NVM_SYMLINK%`
- `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`
- `%NVM_SYMLINK%\node.exe`
- `<repo-root>\scripts`
- `%LOCALAPPDATA%\nvm`
- `%ProgramFiles%\PowerShell\7\pwsh.exe`
- `%ProgramFiles%\Google\Chrome\Application\chrome.exe`
- `%LOCALAPPDATA%\nvm\v20.19.3\node.exe`
- `%SystemRoot%\System32\cmd.exe`
- `<repo-root>\node_modules\.pnpm\@esbuild+win32-x64@0.27.7\node_modules\@esbuild\win32-x64\esbuild.exe`
- `<repo-root>\node_modules`
