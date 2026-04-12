# Architecture Notes

## Part 2 decisions

- The stable protocol version is `tablinq.v1`.
- Rust owns durable types, validation, and CLI/daemon entry points.
- TypeScript owns schema validation, selector-pack parsing, browser adapter code, and the extension skeleton.
- Selector strategies are data in `selector-packs/v1/*.json`, not embedded constants.
- The fake harness is the first browser target for CI-safe tests.
- The daemon speaks a JSON request-response protocol over local sockets: named pipes on Windows through `interprocess`, Unix-domain-style local sockets elsewhere through the same abstraction.
- Persistent state lives under one resolved state root with fixed subdirectories for config, logs, db, browser profile, downloads, and crash bundles.
- SQLite migrations run at daemon boot and back the job, artifact, and session registries from day one.
- Single-instance protection is enforced with a startup lock file plus responsive-daemon checks; stale lock files are reclaimed.

## Protocol surface

- Commands use an envelope with `version`, `request_id`, `session_id`, `sent_at`, and a tagged `command`.
- Events use an envelope with `version`, `request_id`, `emitted_at`, and a tagged `event`.
- Payload shapes are independently versioned under `schemas/v1/`.
- Runtime-discovered labels are represented as alias maps so UI text stays out of core control logic.
- The CLI/daemon control path uses `DaemonRequest` and `DaemonResponse` types in `shared-protocol` for `ping`, `status`, `shutdown`, `doctor`, `config show`, `sessions list`, and `jobs list`.

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

## Selector-pack expectations

Each selector pack must:

- declare `format_version: 1`
- define every known capability
- provide at least one strategy per capability
- keep strategies externally editable so live-site drift can be patched without rebuilding the daemon
