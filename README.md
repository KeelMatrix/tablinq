# Tablinq

**The browser is the provider.**

Tablinq turns a real ChatGPT browser session into a local CLI and an OpenAI-compatible gateway. It keeps the browser-driven architecture on purpose: your managed browser session remains the execution engine, while Rust and TypeScript provide the control plane, persistence, packaging, and safe integrations around it.

If you want local tools, scripts, or agent frameworks to work through a dedicated ChatGPT session without exporting cookies, replaying bearer tokens, or pretending the website is a raw API, Tablinq is built for that.

## Why people care

- **Real browser session, not a fake provider.** Tablinq drives a managed ChatGPT tab through Playwright and an extension bridge.
- **Local-first control plane.** Jobs, conversations, artifacts, failure bundles, and request history stay on your machine.
- **OpenAI-compatible gateway.** Tools such as OpenClaude and Paperclip can talk to Tablinq through familiar `/v1/*` routes.
- **Safe by default.** One warm browser session, one active upstream request, conservative queueing, and no blind auto-replay of interrupted requests.
- **Production-minded recovery.** Interrupted requests can be detected, reconciled against the live conversation, and marked as recovered when the upstream result already exists.
- **CI-safe browser coverage.** A fake ChatGPT harness exercises browser behavior in tests without depending on the live site.

## What Tablinq does

- runs a dedicated managed browser profile for ChatGPT
- exposes local CLI commands for ask, research, jobs, sessions, artifacts, browser lifecycle, and diagnostics
- exposes an OpenAI-compatible local HTTP gateway for browser-backed provider mode
- persists jobs, conversations, artifacts, request summaries, and failure bundles locally
- handles browser disconnects, reloads, stalls, and client disconnects without hanging forever
- keeps selector packs and protocol schemas versioned outside the compiled runtime

## Why this architecture is unusual

Most AI tooling assumes direct API access. Tablinq starts from the opposite assumption:

1. the browser is the source of truth
2. the browser session may be fragile, stateful, and UI-driven
3. local tooling still needs a stable CLI and provider interface

That leads to a different stack:

```mermaid
flowchart LR
    CLI["`gpt` CLI"] --> IPC["local IPC"]
    IPC --> Daemon["`gptd` daemon"]
    Daemon --> DB["SQLite + state root"]
    Daemon --> Adapter["browser adapter (TS + Playwright)"]
    Adapter --> Browser["managed Edge / Chromium profile"]
    Browser --> Extension["MV3 extension"]
    Extension --> Page["ChatGPT page or fake harness"]
    Gateway["OpenAI-compatible HTTP gateway"] --> Daemon
```

## Quick demo

```bash
gpt daemon start
gpt gateway start
gpt browser open
gpt ask "Summarize the value of Tablinq in one paragraph."
gpt research "Investigate launch blockers for a desktop AI tool."
gpt status
```

## Three-minute setup

1. Install Rust, Node.js 20+, and `pnpm`.
2. Run `pnpm install`.
3. Validate the workspace:
   - `cargo test`
   - `pnpm test`
4. Build and install Tablinq:
   - Windows: `scripts/package.ps1` then `scripts/install.ps1`
   - Unix-like: `./scripts/package.sh` then `./scripts/install.sh`
5. Load the unpacked extension from the installed `extension` directory.
6. Start the control plane:
   - `gpt daemon start`
   - `gpt gateway start`
7. Open the managed browser: `gpt browser open`
8. Sign in once in the managed profile.
9. Confirm health:
   - `gpt status`
   - `gpt doctor`
   - `gpt capabilities`

For the full install flow, see [docs/install.md](docs/install.md).

## CLI surface

Common commands:

- `gpt version`
- `gpt status`
- `gpt doctor`
- `gpt capabilities`
- `gpt config show`
- `gpt daemon start|status|stop`
- `gpt gateway start|status|stop`
- `gpt ask "hello"`
- `gpt ask --model instant --mode search "latest release notes"`
- `gpt ask --model thinking --effort extended "compare these options"`
- `gpt ask --file notes.txt "summarize the attachment"`
- `gpt research "investigate launch blockers"`
- `gpt jobs list`
- `gpt jobs show job-123`
- `gpt jobs cancel job-123`
- `gpt sessions list`
- `gpt artifacts list`
- `gpt artifacts pull last`
- `gpt browser open`
- `gpt browser close`
- `gpt browser logout`
- `gpt browser switch-account`
- `gpt browser reset-profile`
- `gpt browser profile-info`

## Provider mode

Tablinq can also run as a local provider for browser-backed automation.

- base URL: `http://127.0.0.1:3847/v1`
- auth: local gateway API key from `config.toml`
- model families:
  - `tablinq-instant`
  - `tablinq-thinking-standard`
  - `tablinq-thinking-extended`
  - `tablinq-search`
  - `tablinq-research`
- recommended concurrency:
  - `max_parallel_requests = 1`
  - `max_parallel_branches = 1`
  - subagents disabled by default

Start with [docs/provider-mode.md](docs/provider-mode.md), then see:

- [docs/openclaude.md](docs/openclaude.md)
- [docs/paperclip.md](docs/paperclip.md)

## Recovery and safety model

Tablinq does **not** rely on hidden network interception, exported cookies, or bearer-token replay.

It stays browser-first and handles failures conservatively:

- explicit request states include `running`, `interrupted`, `recovered`, and `failed`
- local/browser/network interruption is detected with bounded timeouts
- interrupted work is reconciled against the live conversation when possible
- streaming resumes only when the observed page state makes that safe
- prompts are not blindly resubmitted by default

This matters when a browser restarts, a tab reloads, a stream stalls, or a client disconnects while upstream work may still be running.

## Workspace map

- `crates/gpt`: CLI entry point and user-facing command presentation
- `crates/gptd`: daemon service, state/config bootstrap, SQLite, browser orchestration
- `crates/shared-protocol`: stable protocol types and state-path helpers
- `crates/tablinq-gateway`: local OpenAI-compatible HTTP gateway
- `crates/tablinq-host`: native messaging host for the extension
- `packages/browser-adapter`: Playwright-backed browser runtime
- `packages/extension`: MV3 extension bridge and capability probes
- `packages/fake-chatgpt`: fake browser harness for tests
- `schemas/v1`: versioned protocol contracts
- `selector-packs/v1`: selector data kept outside compiled code
- `docs`: install, development, architecture, provider, and troubleshooting docs

## Testing and CI

Local validation:

- Rust: `cargo test`
- TypeScript: `pnpm lint`, `pnpm build`, `pnpm test`
- formatting: `cargo fmt --all --check`, `pnpm format`

GitHub Actions:

- `.github/workflows/ci.yml`
  - Rust formatting, clippy, and workspace tests
  - TypeScript formatting, type-check, build, and tests
- `.github/workflows/package-smoke.yml`
  - Windows packaged install smoke coverage
  - package artifact upload on non-PR runs

## Documentation

- [Install](docs/install.md)
- [Development](docs/development.md)
- [Architecture](docs/architecture.md)
- [Provider mode](docs/provider-mode.md)
- [OpenClaude](docs/openclaude.md)
- [Paperclip](docs/paperclip.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Live smoke checklist](docs/live-smoke-checklist.md)

## Contributing

Before opening a PR, match the project’s normal validation surface:

- `cargo test`
- `pnpm lint`
- `pnpm build`
- `pnpm test`

If your change affects packaging or installed behavior on Windows, also exercise the package smoke path.
