# Provider Mode

Tablinq can run as a local browser-backed provider for OpenAI-compatible clients, OpenClaude, and Paperclip. The safe default is one warm browser session, one active upstream request, no subagents.

## Package And Install

1. Build a local package with `scripts/package.ps1` or `./scripts/package.sh`.
2. Install it with `scripts/install.ps1` or `./scripts/install.sh`.
3. Load the unpacked extension from the installed `extension` directory.
4. Start the control plane:
   - `gpt daemon start`
   - `gpt gateway start`
5. Open the managed browser once with `gpt browser open`.
6. Sign in once in the managed profile.
7. Check the merged operational view with `gpt status` or `gpt doctor`.

## OpenClaude

- Base URL: `http://127.0.0.1:3847/v1`
- API key: `gateway.api_key` from `config.toml`
- Streaming: `true`
- Max parallel requests: `1`
- Recommended models:
  - `tablinq-instant`
  - `tablinq-thinking-standard`
  - `tablinq-search`
- Request annotations:
  - `tablinq_request_role = "primary"`
  - `tablinq_request_role = "branch"`
  - `tablinq_request_role = "subagent"`
  - `tablinq_parent_request_id = "<request-id>"`

## Paperclip

- Base URL: `http://127.0.0.1:3847/v1`
- API key: `gateway.api_key` from `config.toml`
- Transport: OpenAI-compatible HTTP
- Max parallel requests: `1`
- Recommended models:
  - `tablinq-instant`
  - `tablinq-search`
  - `tablinq-research`
- File flow:
  1. `POST /v1/files`
  2. pass staged IDs to `POST /v1/chat/completions`
  3. read `GET /v1/artifacts`
  4. pull `GET /v1/artifacts/{id}/content`

## Safe Parallelism

- Keep `max_parallel_requests = 1`.
- Keep `max_parallel_branches = 1`.
- Keep subagents disabled by default.
- Respect `retry_after_ms` on `429` responses.
- Prefer one warm browser session over multiple profiles.

## Status, Logs, And Recovery

- `gpt status` or `gpt doctor`: merged daemon, gateway, browser, auth, extension, queue, and log paths.
- `gpt daemon status`: daemon-only status.
- `gpt gateway status`: gateway-only status.
- Logs:
  - daemon: `<state-root>/logs/gptd.log`
  - gateway: `<state-root>/logs/gateway.log`
- Recovery commands:
  - `gpt browser open`
  - `gpt browser reset-profile`
  - `gpt gateway stop`
  - `gpt daemon stop`
  - `gpt daemon start`
  - `gpt gateway start`

## Resetting The Managed Profile

Use `gpt browser reset-profile` when the managed browser profile is wedged, stale, or signed into the wrong account. This clears the managed profile directory under `<state-root>/browser-profile`. You will need to reopen the browser and sign in again afterward.
