# Architecture Notes

## Part 1 decisions

- The stable protocol version is `tablinq.v1`.
- Rust owns durable types, validation, and CLI/daemon entry points.
- TypeScript owns schema validation, selector-pack parsing, browser adapter code, and the extension skeleton.
- Selector strategies are data in `selector-packs/v1/*.json`, not embedded constants.
- The fake harness is the first browser target for CI-safe tests.

## Protocol surface

- Commands use an envelope with `version`, `request_id`, `session_id`, `sent_at`, and a tagged `command`.
- Events use an envelope with `version`, `request_id`, `emitted_at`, and a tagged `event`.
- Payload shapes are independently versioned under `schemas/v1/`.
- Runtime-discovered labels are represented as alias maps so UI text stays out of core control logic.

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
