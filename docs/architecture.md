# Architecture

Tablinq is intentionally browser-driven. The browser is not a side effect or fallback. It is the execution engine, and the rest of the system exists to make that execution model durable, inspectable, and integrable.

## Ownership boundaries

- Rust owns durable types, validation, the CLI, daemon, gateway, config, logging, SQLite persistence, and package/install flow.
- TypeScript owns the browser adapter, extension bridge, capability probes, and fake browser harness.
- Selector strategies live in `selector-packs/v1/*.json` instead of being compiled into the adapter.
- Stable protocol and schema contracts live under `crates/shared-protocol` and `schemas/v1`.

## Runtime topology

1. `gpt` parses local user commands and sends daemon requests over local IPC.
2. `gptd` owns durable execution, persistence, request lifecycle state, and one supervised browser adapter process.
3. `tablinq-gateway` exposes an OpenAI-compatible HTTP surface backed by the same core service.
4. The browser adapter drives one managed browser profile with Playwright.
5. The extension probes the page and publishes capability and alias data back to the browser-driven control loop.

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

The IPC endpoint is derived from the state root so the CLI and daemon agree on the same local socket name without extra registration.

## Browser runtime

- The daemon owns the managed browser session and dedicated profile.
- The CLI never talks to the browser directly.
- The browser adapter is launched from packaged `service.js` output when present and falls back to the repo `service.ts` entry in a developer checkout.
- The adapter keeps one managed page alive, recreates it after closes or crashes, and reports `BrowserStatusSnapshot` health back to the daemon.
- Live-site DOM helpers stay isolated in `packages/browser-adapter/src/pageAdapter.runtime.js` so UI-specific heuristics do not leak across the long-lived control loop.

## Request lifecycle

Ask and research requests move through explicit states:

- `queued`
- `starting`
- `running`
- `interrupted`
- `recovered`
- terminal states such as `completed`, `failed`, or `cancelled`

That state is persisted and surfaced through the CLI and gateway so clients can distinguish:

- still running work
- interrupted work that may still be active upstream
- recovered work whose final result was reconciled after interruption
- unrecoverable failures

## Ask and research flow

- `gpt ask` becomes `DaemonRequest::Ask`, which streams `EventPayload` frames followed by a final `DaemonResponse`.
- `gpt research` becomes `DaemonRequest::Research`, which runs through a durable job path with progress updates and artifact persistence.
- The daemon persists jobs, sessions, conversations, request summaries, artifacts, and failure bundles in SQLite and the state root.
- The gateway reuses the same core service instead of duplicating execution logic.

## Capability and selector model

- The extension publishes a capability matrix plus runtime alias maps for visible labels.
- The control plane uses those alias maps instead of hardcoding current UI text for model, mode, and effort selection.
- Strategy success and failure telemetry are persisted so the managed profile can reuse last-known-good selectors per workspace.

## Interruption and recovery model

Tablinq assumes browser-backed execution can break in ways a normal API caller does not:

- browser disconnect
- tab reload or navigation
- client disconnect while upstream work may still continue
- streaming stalls

Recovery is conservative:

- requests are detected as interrupted with bounded timeouts
- the adapter reattaches to the managed session when possible
- the live conversation is inspected before any continuation
- a request is marked `recovered` only when the observed assistant result matches the interrupted request safely
- prompts are not blindly resubmitted by default

## Failure handling

Typed failures are surfaced for:

- authentication problems
- capability gaps
- profile locks
- browser launch issues
- upload and download failures
- action timeouts
- research-specific failures
- reconciliation failures after interruption

Failure bundles are written under `crash-bundles/` and recorded in SQLite with screenshot, DOM snapshot, ARIA snapshot, active URL, selector attempts, and linked logs.

## HTTP gateway

`tablinq-gateway` exposes OpenAI-compatible endpoints for:

- chat completions
- files
- artifacts
- request status and cancellation
- sessions and provider metadata

The gateway keeps browser usage stable with queueing and concurrency controls rather than pretending the system is horizontally scalable.
