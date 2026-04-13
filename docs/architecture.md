# Architecture Notes

## Part 11 decisions

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
- The browser adapter is a long-lived TypeScript service launched from a packaged `service.js` entry when present, or through `npx tsx` in a developer checkout.
- Browser health is surfaced to the daemon as a `BrowserStatusSnapshot`, which now includes `extension_connected` and the last `CapabilityReport`.
- Ask execution is streamed over the local daemon socket as `EventPayload` frames followed by a final `DaemonResponse`.
- Runtime alias and last-known-good strategy data are cached per normalized workspace URL inside the managed browser profile so selection can reuse the latest discovered labels.
- Strategy success and failure counts are now persisted per capability and strategy alongside the workspace capability cache.
- File attachments are passed as local paths from the CLI to the daemon and then attached through Playwright.
- Artifact downloads are captured by Playwright, moved into managed storage, and registered in SQLite with checksums plus conversation/message linkage.
- Research runs as a dedicated long-running job path instead of being treated as an ordinary ask.
- Job details, cancellation requests, and job-to-artifact mappings are persisted in SQLite alongside the existing job summary table.
- Failure bundles are persisted both as SQLite records and as on-disk crash-bundle directories containing DOM, ARIA, and screenshot artifacts.
- Packaged installs now use `bin/`, `browser-adapter/src/`, `extension/`, `selector-packs/`, and `schemas/` as the stable runtime layout.
- Windows installs register `tablinq-host` as a user-level native messaging host and keep both state and packaged artifacts under `%LOCALAPPDATA%\\Tablinq`.
- Live-site page actions are isolated in `packages/browser-adapter/src/pageAdapter.runtime.js` so current ChatGPT-specific control heuristics stay out of the long-lived Playwright control loop.
- The adapter now restores a usable managed page after artifact-triggered frame loss by recreating the page and retargeting the active conversation URL when needed.
- Timeout policy is explicit and conservative: navigation, probing, interaction-surface waits, upload acknowledgement, ask completion, research completion, and download capture each use dedicated limits instead of one shared timeout.

## Protocol surface

- Commands use an envelope with `version`, `request_id`, `session_id`, `sent_at`, and a tagged `command`.
- Events use an envelope with `version`, `request_id`, `emitted_at`, and a tagged `event`.
- Payload shapes are independently versioned under `schemas/v1/`.
- Runtime-discovered labels are represented as alias maps so UI text stays out of core control logic.
- The CLI/daemon control path uses `DaemonRequest` and `DaemonResponse` types in `shared-protocol` for `ask`, `research`, `artifacts list`, `artifacts pull`, `ping`, `status`, `shutdown`, `doctor`, `capabilities`, `config show`, `sessions list`, `jobs list|show|cancel`, and browser lifecycle commands.
- The daemon/browser path uses a smaller JSON-line adapter protocol over stdio for `open`, `health`, `diagnostics`, `ask`, `research`, `close`, and `shutdown`, with capability probes piggybacking on browser health snapshots.
- For streamed asks, the daemon/browser adapter path emits `stream_chunk` frames until an `ask_result` frame closes the request.
- Ask requests now forward `mode`, `model_class`, `effort`, and `files` to the browser adapter before composer submission.

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
- The adapter evaluates the extension probe module inside the managed page, loads selector-pack metadata from external JSON, and keeps last-known-good strategy choices per observed UI signature.
- The adapter now owns prompt insertion, submit clicks, DOM-stream observation, timeout detection, conversation/message capture, control selection for model/mode/effort requests, and post-download page recovery.
- In-page ask and research reads now flow through the dedicated runtime module rather than large embedded source strings in `browserAdapter.ts`.
- The daemon retries recoverable ask/research failures once by relaunching the adapter and recreating the managed tab.
- Safe mode is a config-level execution guard that keeps probes and doctor working while blocking ask and research submission.
- `doctor` now includes daemon health, browser detection, profile access checks, selector-pack metadata, strategy telemetry, recent failure-bundle summary, and recovery guidance.

## Ask flow

The Part 9 ask path stays local and DOM-driven:

- `gpt ask` sends a streamed `DaemonRequest::Ask`.
- `gptd` creates a durable job row, resumes the last conversation when no explicit conversation is provided, and forwards stream events to the CLI.
- The browser adapter refreshes the latest capability report, applies requested model/mode/effort settings from discovered aliases, attaches requested files through Playwright, confirms page acknowledgement, focuses the composer, inserts the prompt, clicks submit, and polls the managed page for DOM text growth.
- Streamed deltas are normalized into `StreamChunk` events and forwarded immediately to the terminal.
- Completion stores job state, the latest conversation mapping, and any downloaded artifact metadata in SQLite.
- Missing requested controls fail fast as `capability_missing`; mismatches between a visible capability and the action layer are classified as `ui_drift`.
- Upload acknowledgement failures surface as `upload_failed`; download capture failures surface as `download_failed`.
- Ask failures now also create a local failure bundle with screenshot, DOM snapshot, ARIA snapshot, capability matrix, strategy attempts, and log linkage.

## Research flow

- `gpt research` sends a streamed `DaemonRequest::Research`.
- `gptd` creates a durable research job row immediately, then runs the browser work on a dedicated worker thread so the main IPC loop can still answer `jobs show` and `jobs cancel`.
- The browser adapter enters research mode, submits the query, observes explicit research progress states from the page, streams report deltas as they appear, and collects final artifacts.
- Research progress is persisted as job detail state, including partial output text, current page URL, cancellation requests, and linked artifacts.
- Cancellation is best-effort: the daemon marks the job as cancel-requested and tears down the in-flight adapter process on the next progress checkpoint.
- Research failures now produce the same local failure-bundle diagnostics as asks.

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
- a live-like variant with `#prompt-textarea`, send/stop button toggles, current-style model/mode controls, upload chips, and artifact downloads

It also exposes Part 8 control, upload, download, research, and streaming scenarios:

- model and effort groups with renamed labels
- optional missing search/model/effort controls
- upload acknowledgement via `data-slot` markers and `body.dataset.uploadedFiles`
- artifact links that become download-ready only after completion
- successful streamed output
- stalled output for timeout handling
- one forced reload during an active stream
- research progress with `waiting_for_sources` -> `running` -> `done`
- research failure with a retryable-looking error

Part 9 adds verification on top of those scenarios for:

- selector-pack driven probe ordering
- failure-bundle creation on timeout and other adapter-visible failures
- doctor reporting of selector-pack and telemetry state
- recovery guidance for browser launch, auth, profile access, and probe drift cases

Part 11 extends that verification with:

- runtime alias mapping for current ChatGPT-style visible labels and controls
- live-like prompt/submit/upload/download behavior driven through the isolated page runtime module
- artifact-download recovery checks that keep the managed page usable after a download invalidates the active frame
- an optional manual smoke checklist for real-site login, ask, search, thinking, upload, download, and research flows

## Selector-pack expectations

Each selector pack must:

- declare `format_version: 1`
- define every known capability
- provide at least one strategy per capability
- keep strategies externally editable so live-site drift can be patched without rebuilding the daemon
- keep stable strategy identifiers so telemetry and failure bundles stay comparable across runs
