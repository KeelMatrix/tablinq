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
- `cargo run -p gpt -- browser start-debug`
- `cargo run -p gpt -- browser attach`
- `cargo run -p gpt -- browser connection-info`
- `cargo run -p gpt -- browser open`
- `cargo run -p gpt -- browser close`
- `cargo run -p gpt -- browser logout`
- `cargo run -p gpt -- browser switch-account`
- `cargo run -p gpt -- browser reset-profile`
- `cargo run -p gpt -- browser profile-info`

## Local browser workflow for real-site checks

Use this only when the fake harness is not enough:

1. Start the daemon and gateway.
2. Load the unpacked extension into your normal browser profile.
3. Fully close the browser.
4. Run `cargo run -p gpt -- browser start-debug`.
5. Sign in manually at `https://chatgpt.com/`.
6. Run `cargo run -p gpt -- browser attach`.
7. Use `cargo run -p gpt -- status`, `doctor`, and `capabilities` before testing ask or research flows.

Managed fallback exists only for debugging, CI, harness isolation, and machines that cannot expose a usable local debug endpoint.

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
  - CLI command surface and user-facing presentation
- `crates/gptd/src/core.rs`
  - durable execution logic, state transitions, and persistence boundaries
- `crates/gptd/src/browser.rs`
  - browser-adapter supervision and recovery handoff
- `crates/tablinq-gateway/src/lib.rs`
  - HTTP provider surface, queueing, and request summaries
- `packages/browser-adapter/src/browserAdapter.ts`
  - Playwright attach runtime, browser ownership boundaries, and reconciliation logic
- `packages/browser-adapter/src/pageAdapter.runtime.js`
  - page-level DOM helpers and UI-specific logic
- `packages/extension/src/probe.ts`
  - capability probes and alias discovery

## Antivirus warning on Windows

If packaging, tests, or browser startup fail with `Access is denied`, check antivirus notifications before debugging the repo. On some Windows machines, local toolchain executables such as `node.exe`, `pwsh.exe`, `chrome.exe`, `cmd.exe`, or `esbuild.exe` may need explicit antivirus exceptions.

Common exception candidates:

- `C:\Users\user\Desktop\gpt-cli`
- `C:\Users\user\AppData\Local\Tablinq`
- `C:\Users\user\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup`
- `C:\nvm4w\nodejs`
- `C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe`
- `C:\nvm4w\nodejs\node.exe`
- `C:\Users\user\Desktop\gpt-cli\scripts`
- `C:\Users\user\AppData\Local\nvm`
- `C:\Program Files\PowerShell\7\pwsh.exe`
- `C:\Program Files\Google\Chrome\Application\chrome.exe`
- `C:\Users\user\AppData\Local\nvm\v20.19.3\node.exe`
- `C:\Windows\System32\cmd.exe`
- `C:\Users\user\Desktop\gpt-cli\node_modules\.pnpm\@esbuild+win32-x64@0.27.7\node_modules\@esbuild\win32-x64\esbuild.exe`
- `C:\Users\user\Desktop\gpt-cli\node_modules`
