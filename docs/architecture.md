# Architecture

## Ownership boundaries

- Rust owns durable types, validation, the CLI, daemon, gateway, config, logging, and SQLite persistence.
- TypeScript owns the browser adapter, extension bridge, capability probes, and fake browser harness.
- Selector strategies live in `selector-packs/v1/*.json` instead of being compiled into the adapter.
- JSON protocol and schema contracts live under `crates/shared-protocol` and `schemas/v1`.

## Runtime topology

1. `gpt` parses CLI arguments and sends local daemon requests.
2. `gptd` persists state, supervises one browser adapter process, and streams ask or research output back to the caller.
3. `tablinq-gateway` exposes an OpenAI-compatible HTTP API backed by the same core service.
4. The browser adapter drives one managed browser profile with Playwright.
5. The extension probes the page and reports capability snapshots back to the daemon path.

## State layout

The resolved state root contains:

- `config/config.toml`
- `logs/gptd.log`
- `logs/gateway.log`
- `db/tablinq.sqlite3`
- `browser-profile/`
- `downloads/`
- `crash-bundles/`
- `gateway-uploads/`
- `daemon.lock`

The IPC endpoint is derived from the resolved state root so the CLI and daemon agree on the local socket name without extra registration.

## Browser runtime

- The daemon owns the managed browser session and dedicated profile.
- The CLI never talks to the browser directly.
- The browser adapter is launched from packaged `service.js` output when present, and falls back to the repo `service.ts` entry in a developer checkout.
- The adapter keeps one managed page alive, recreates it after crashes or closes, and reports `BrowserStatusSnapshot` health back to the daemon.
- Live-site DOM helpers are isolated in `packages/browser-adapter/src/pageAdapter.runtime.js` so site-specific heuristics stay outside the long-lived Playwright control loop.

## Ask and research flow

- `gpt ask` becomes `DaemonRequest::Ask`, which streams `EventPayload` frames followed by a final `DaemonResponse`.
- `gpt research` becomes `DaemonRequest::Research`, which runs through a durable job path with progress states.
- The daemon persists jobs, sessions, conversations, artifacts, and failure bundles in SQLite and the state root.
- Recoverable adapter failures are retried once by relaunching the adapter and recreating the managed tab.

## Capability and selector model

- The extension publishes a capability matrix plus runtime alias maps for visible labels.
- The daemon uses alias maps rather than hardcoding current UI text for model, mode, and effort selections.
- Strategy success and failure telemetry are persisted so the managed profile can reuse last-known-good selectors per workspace.

## Failure handling

- Typed errors are surfaced for auth, capability gaps, profile locks, browser launch issues, download failures, upload failures, timeout failures, and research failures.
- Failure bundles are written under `crash-bundles/` and recorded in SQLite with screenshot, DOM snapshot, ARIA snapshot, active URL, strategy attempts, and logs linkage.

## HTTP gateway

- `tablinq-gateway` exposes OpenAI-compatible endpoints for chat completions, files, artifacts, status, and provider metadata.
- The gateway uses the same core service as the CLI and daemon instead of duplicating execution logic.
- Safe defaults are enforced around queueing and parallelism so one managed browser session stays stable.
