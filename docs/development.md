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
- `cargo run -p gpt -- ask "hello world"`
- `cargo run -p gpt -- ask --model instant --mode search "latest release notes"`
- `cargo run -p gpt -- ask --model thinking --effort extended "compare these ideas"`
- `cargo run -p gpt -- ask --file notes.txt --file data.csv "summarize uploads"`
- `cargo run -p gpt -- research "investigate launch blockers" --file notes.txt`
- `cargo run -p gpt -- artifacts list`
- `cargo run -p gpt -- artifacts pull last`
- `cargo run -p gpt -- jobs show job-123`
- `cargo run -p gpt -- jobs cancel job-123`
- `cargo run -p gpt -- doctor`
- `cargo run -p gpt -- capabilities`
- `cargo run -p gpt -- config show`
- `cargo run -p gpt -- browser open`
- `cargo run -p gpt -- browser profile-info`
- `cargo run -p gpt -- browser close`
- `pnpm test`
- `pnpm lint`
- `pnpm build`

## Windows-first helper scripts

- `scripts/bootstrap.ps1`: checks the local toolchain and adds the user Rust install to `PATH` for the current PowerShell session when needed.
- `scripts/test.ps1`: runs `cargo test` and `pnpm test` in one step.
- `scripts/package.ps1`: builds release Rust binaries, emits the browser adapter and MV3 extension JavaScript bundles, and assembles `dist/package/tablinq`.
- `scripts/install.ps1`: copies the packaged layout into `%LOCALAPPDATA%\Tablinq\install`, initializes `%LOCALAPPDATA%\Tablinq`, verifies Edge/Chrome, and registers the native messaging host in the user hive.

## Current testing strategy

- Rust tests cover protocol serialization, daemon boot, IPC connectivity, config bootstrap, SQLite migrations, job persistence across restart, stale-lock recovery, browser health messaging, streamed ask execution, timeout diagnostics, failure-bundle persistence, capability snapshots, configured ask requests, capability-missing failures, upload/download artifact persistence, research success/failure/cancellation, and CLI parsing.
- TypeScript tests validate the JSON schemas, probe the extension against synthetic DOM fixtures, and exercise the browser adapter against the classic, ARIA-first, and live-like `fake-chatgpt` variants plus selector-pack loading, diagnostics capture, streaming/reload, renamed-control, missing-control, upload, managed-download, recovery-after-download, and research-progress scenarios with real Playwright flows.
- `packages/fake-chatgpt` is the CI-safe target for browser and extension fallback tests.
- Optional real-site verification is documented in [docs/live-smoke-checklist.md](/C:/Users/user/Desktop/gpt-cli/docs/live-smoke-checklist.md) and stays outside CI.

## Current execution paths

Part 11 finishes the hardening and live-adaptation pass:

- `packages/extension/src/probe.ts`: ARIA-aware capability modules, strategy scoring, alias discovery, and DOM neighborhood snapshots.
- `packages/extension/src/content-script.ts`: debounced capability publishing on DOM changes.
- `packages/extension/src/service-worker.ts`: native-messaging and development WebSocket forwarding.
- `packages/browser-adapter/src/browserAdapter.ts`: selector-pack loading, capability-cache persistence, strategy telemetry, runtime model/mode/effort selection, Playwright-backed uploads, managed downloads, prompt insertion, submit, streamed DOM observation, explicit timeout handling, diagnostics capture, reload recovery, and post-download page restoration.
- `packages/browser-adapter/src/pageAdapter.runtime.js`: isolated live-site DOM helpers for composer readiness, prompt filling, submit, ask/research snapshot reads, configuration controls, and upload acknowledgement.
- `gpt ask`: streamed terminal output backed by daemon job persistence, last-conversation resume, and returned artifact metadata.
- `gpt research`: streamed report output backed by a dedicated durable job record, partial-output persistence, and optional local report writing.
- `gpt capabilities`: daemon-facing inspection of the last real capability matrix and alias set from the managed page.
- `gpt artifacts list|pull`: SQLite-backed artifact inspection and managed download retrieval.
- `gpt jobs show|cancel`: persisted job detail inspection and best-effort cancellation signaling.
- `gpt doctor`: daemon status plus browser detection, profile access, selector-pack summary, strategy telemetry, recent failure-bundle summary, and recovery guidance.
- Capability-missing requests now fail fast with a typed daemon error instead of silently guessing.
- Upload, download, research, timeout, auth-required, profile-lock, launch, and drift failures are surfaced as typed daemon errors and bundled local diagnostics.
- `config.toml` now supports `safe_mode = true` for probe-only local debugging.

## Implementation status

Part 11 is now implemented:

- package builds now emit runnable JavaScript assets for `packages/browser-adapter` and `packages/extension`
- `gptd` prefers the packaged `service.js` adapter entry and falls back to the repo `service.ts` entry for developer checkouts
- a dedicated `tablinq-host` binary is packaged and registered as the Windows user-level native messaging host
- `gpt version` and `gptd version` expose build metadata for packaged installs
- end-user docs now cover install, login, troubleshooting, selector-pack updates, and profile reset
- current ChatGPT-style selectors and alias heuristics live in selector packs, probe modules, and the isolated page runtime instead of the core Playwright loop
- ask and research now use explicit timeout/retry policies for navigation, probing, upload acknowledgement, DOM streaming, and download capture
- managed downloads now recover the page when a browser download invalidates the active frame, leaving the session usable for follow-up commands
- the repo includes a manual live smoke checklist for final local verification without adding live-site dependencies to CI
