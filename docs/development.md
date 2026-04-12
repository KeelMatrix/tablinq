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

## Current testing strategy

- Rust tests cover protocol serialization, daemon boot, IPC connectivity, config bootstrap, SQLite migrations, job persistence across restart, stale-lock recovery, browser health messaging, streamed ask execution, timeout diagnostics, failure-bundle persistence, capability snapshots, configured ask requests, capability-missing failures, upload/download artifact persistence, research success/failure/cancellation, and CLI parsing.
- TypeScript tests validate the JSON schemas, probe the extension against synthetic DOM fixtures, and exercise the browser adapter against both `fake-chatgpt` UI variants plus selector-pack loading, diagnostics capture, streaming/reload, renamed-control, missing-control, upload, managed-download, and research-progress scenarios with real Playwright flows.
- `packages/fake-chatgpt` is the CI-safe target for browser and extension fallback tests.

## Current execution paths

Part 9 hardens the execution paths around drift and failure recovery:

- `packages/extension/src/probe.ts`: ARIA-aware capability modules, strategy scoring, alias discovery, and DOM neighborhood snapshots.
- `packages/extension/src/content-script.ts`: debounced capability publishing on DOM changes.
- `packages/extension/src/service-worker.ts`: native-messaging and development WebSocket forwarding.
- `packages/browser-adapter/src/browserAdapter.ts`: selector-pack loading, capability-cache persistence, strategy telemetry, runtime model/mode/effort selection, Playwright-backed uploads, managed downloads, prompt insertion, submit, streamed DOM observation, timeout handling, diagnostics capture, and reload recovery.
- `gpt ask`: streamed terminal output backed by daemon job persistence, last-conversation resume, and returned artifact metadata.
- `gpt research`: streamed report output backed by a dedicated durable job record, partial-output persistence, and optional local report writing.
- `gpt capabilities`: daemon-facing inspection of the last real capability matrix and alias set from the managed page.
- `gpt artifacts list|pull`: SQLite-backed artifact inspection and managed download retrieval.
- `gpt jobs show|cancel`: persisted job detail inspection and best-effort cancellation signaling.
- `gpt doctor`: daemon status plus browser detection, profile access, selector-pack summary, strategy telemetry, recent failure-bundle summary, and recovery guidance.
- Capability-missing requests now fail fast with a typed daemon error instead of silently guessing.
- Upload, download, research, timeout, auth-required, profile-lock, launch, and drift failures are surfaced as typed daemon errors and bundled local diagnostics.
- `config.toml` now supports `safe_mode = true` for probe-only local debugging.

## Next implementation boundary

Part 10 should package the now-hardened local system for easier Windows-first installation:

- packaging and install scripts
- native messaging/bootstrap polish
- end-user install and troubleshooting docs
