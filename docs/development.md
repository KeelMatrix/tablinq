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

- Rust tests cover protocol serialization, daemon boot, IPC connectivity, config bootstrap, SQLite migrations, job persistence across restart, stale-lock recovery, browser health messaging, streamed ask execution, timeout diagnostics, capability snapshots, configured ask requests, capability-missing failures, upload/download artifact persistence, research success/failure/cancellation, and CLI parsing.
- TypeScript tests validate the JSON schemas, probe the extension against synthetic DOM fixtures, and exercise the browser adapter against both `fake-chatgpt` UI variants plus streaming/reload, renamed-control, missing-control, upload, managed-download, and research-progress scenarios with real Playwright flows.
- `packages/fake-chatgpt` is the CI-safe target for browser and extension fallback tests.

## Current execution paths

Part 8 adds deep research as a durable job path:

- `packages/extension/src/probe.ts`: ARIA-aware capability modules, strategy scoring, alias discovery, and DOM neighborhood snapshots.
- `packages/extension/src/content-script.ts`: debounced capability publishing on DOM changes.
- `packages/extension/src/service-worker.ts`: native-messaging and development WebSocket forwarding.
- `packages/browser-adapter/src/browserAdapter.ts`: capability-cache persistence, runtime model/mode/effort selection, Playwright-backed uploads, managed downloads, prompt insertion, submit, streamed DOM observation, timeout handling, reload recovery, and research progress polling.
- `gpt ask`: streamed terminal output backed by daemon job persistence, last-conversation resume, and returned artifact metadata.
- `gpt research`: streamed report output backed by a dedicated durable job record, partial-output persistence, and optional local report writing.
- `gpt capabilities`: daemon-facing inspection of the last real capability matrix and alias set from the managed page.
- `gpt artifacts list|pull`: SQLite-backed artifact inspection and managed download retrieval.
- `gpt jobs show|cancel`: persisted job detail inspection and best-effort cancellation signaling.
- Capability-missing requests now fail fast with a typed daemon error instead of silently guessing.
- Upload, download, and research failures are surfaced as typed daemon errors.

## Next implementation boundary

Part 9 should harden diagnostics and recovery around the now-complete core execution paths:

- failure bundles with screenshots and DOM context
- selector-pack drift diagnostics
- recovery guidance and browser relaunch flows
- richer `doctor` coverage
