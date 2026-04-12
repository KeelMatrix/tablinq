# Architecture Notes

## Part 6 decisions

- The stable protocol version is `tablinq.v1`.
- Rust owns durable types, validation, and CLI/daemon entry points.
- TypeScript owns schema validation, selector-pack parsing, browser adapter code, and the extension bridge/runtime probes.
- Selector strategies are data in `selector-packs/v1/*.json`, not embedded constants.
- The fake harness is the first browser target for CI-safe tests.
- The daemon speaks a JSON request-response protocol over local sockets: named pipes on Windows through `interprocess`, Unix-domain-style local sockets elsewhere through the same abstraction.
- Persistent state lives under one resolved state root with fixed subdirectories for config, logs, db, browser profile, downloads, and crash bundles.
- SQLite migrations run at daemon boot and back the job, artifact, and session registries from day one.
- Single-instance protection is enforced with a startup lock file plus responsive-daemon checks; stale lock files are reclaimed.
- The daemon owns one browser adapter child process and talks to it over JSON lines on stdio.
- The browser adapter is a long-lived TypeScript service launched through `npx tsx` and backed by Playwright persistent contexts.
- Browser health is surfaced to the daemon as a `BrowserStatusSnapshot`, which now includes `extension_connected` and the last `CapabilityReport`.
- Ask execution is streamed over the local daemon socket as `EventPayload` frames followed by a final `DaemonResponse`.
- Runtime alias and last-known-good strategy data are cached per normalized workspace URL inside the managed browser profile so selection can reuse the latest discovered labels.

## Protocol surface

- Commands use an envelope with `version`, `request_id`, `session_id`, `sent_at`, and a tagged `command`.
- Events use an envelope with `version`, `request_id`, `emitted_at`, and a tagged `event`.
- Payload shapes are independently versioned under `schemas/v1/`.
- Runtime-discovered labels are represented as alias maps so UI text stays out of core control logic.
- The CLI/daemon control path uses `DaemonRequest` and `DaemonResponse` types in `shared-protocol` for `ask`, `ping`, `status`, `shutdown`, `doctor`, `capabilities`, `config show`, `sessions list`, `jobs list`, and browser lifecycle commands.
- The daemon/browser path uses a smaller JSON-line adapter protocol over stdio for `open`, `health`, `close`, and `shutdown`, with capability probes piggybacking on browser health snapshots.
- For streamed asks, the daemon/browser adapter path emits `stream_chunk` frames until an `ask_result` frame closes the request.
- Ask requests now forward `mode`, `model_class`, and `effort` to the browser adapter before composer submission.

## State layout

The resolved state root contains:

- `config/config.toml`
- `logs/gptd.log`
- `db/tablinq.sqlite3`
- `browser-profile/`
- `downloads/`
- `crash-bundles/`
- `daemon.lock`

The IPC endpoint is derived deterministically from the state root, so the CLI and daemon resolve the same socket name without extra registration.

## Browser runtime

- The dedicated profile directory is always `browser-profile/` under the resolved state root.
- The adapter launches Edge by default on Windows through Playwright’s `msedge` channel.
- On non-Windows platforms, the adapter falls back to detected Chromium-compatible executables or an explicit configured browser path.
- The adapter keeps one managed page alive, recreates it if the tab is closed or crashes, and tracks top-level reloads for diagnostics.
- The adapter evaluates the extension probe module inside the managed page and keeps last-known-good strategy choices per observed UI signature.
- The adapter now owns prompt insertion, submit clicks, DOM-stream observation, timeout detection, conversation/message capture, and control selection for model/mode/effort requests.
- `doctor` now includes daemon health, the current browser snapshot, and the latest capability report.

## Ask flow

The Part 6 ask path stays local and DOM-driven:

- `gpt ask` sends a streamed `DaemonRequest::Ask`.
- `gptd` creates a durable job row, resumes the last conversation when no explicit conversation is provided, and forwards stream events to the CLI.
- The browser adapter refreshes the latest capability report, applies requested model/mode/effort settings from discovered aliases, focuses the composer, inserts the prompt, clicks submit, and polls the managed page for DOM text growth.
- Streamed deltas are normalized into `StreamChunk` events and forwarded immediately to the terminal.
- Completion stores job state plus the latest conversation mapping in SQLite.
- Missing requested controls fail fast as `capability_missing`; mismatches between a visible capability and the action layer are classified as `ui_drift`.

## Capability matrix

The capability matrix is the daemon-facing summary published by the extension/content layer:

- `composer_ready`
- `submit_available`
- `model_picker_visible`
- `thinking_effort_visible`
- `search_mode_visible`
- `research_mode_visible`
- `upload_control_visible`
- `downloadable_artifacts_observable`

The extension side is split into two layers:

- `src/content-script.ts`: DOM observation, debounced capability publishing, and page-side report emission.
- `src/service-worker.ts`: bridge transport selection and forwarding through native messaging or a development WebSocket.

Probe logic lives in `src/probe.ts` as capability modules with multiple strategies. Each probe run emits:

- a protocol-shaped capability report
- a UI signature for local strategy telemetry reuse
- DOM neighborhood snapshots around matched controls for later failure bundles

The fake harness now exposes two intentionally different UI shapes:

- a classic variant with grouped `data-slot` anchors
- an ARIA-first variant with renamed controls and moved upload/composer affordances

It also exposes Part 6 control and streaming scenarios:

- model and effort groups with renamed labels
- optional missing search/model/effort controls
- successful streamed output
- stalled output for timeout handling
- one forced reload during an active stream

## Selector-pack expectations

Each selector pack must:

- declare `format_version: 1`
- define every known capability
- provide at least one strategy per capability
- keep strategies externally editable so live-site drift can be patched without rebuilding the daemon
