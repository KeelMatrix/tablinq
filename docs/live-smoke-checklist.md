# Live Smoke Checklist

This checklist stays out of CI. Use it when you want to validate the attach-first normal-browser flow against the real ChatGPT site.

## Preconditions

- install locally with `scripts/install.ps1` on Windows or `./scripts/package.sh && ./scripts/install.sh` on Unix-like systems
- start the daemon with `gpt daemon start`
- start the gateway with `gpt gateway start`
- load the unpacked Tablinq extension into the profile you will attach to
- when using Chrome attach-first, use the dedicated Chrome attach profile under the Tablinq state root
- fully close that browser
- start the attachable browser session with `gpt browser start-debug`
- sign in manually to `https://chatgpt.com/` in that browser profile
- run `gpt browser attach`
- run `gpt browser connection-info`

Confirm `gpt doctor` reports:

- a running daemon
- a reachable attach endpoint
- extension connectivity
- authentication
- a non-empty capability summary when the page is ready

Also confirm:

- `GET /health`
- `GET /ready`
- `GET /v1/provider`

## CLI smoke checks

1. Run `gpt ask "hello from tablinq"` and confirm streamed output appears.
2. Run `gpt ask --model instant "summarize this in one sentence"` and confirm completion or a clean `capability_missing` failure.
3. Run `gpt ask --model thinking --effort standard "compare rust and typescript for local tooling"` and confirm completion or a clean capability failure.
4. Run `gpt ask --model thinking --effort extended "reason carefully about this tradeoff"` and confirm completion or a clean capability failure.
5. Run `gpt ask --mode search "latest rust stable release"` and confirm completion or a clean capability failure.
6. Run `gpt ask --file path\to\notes.txt "summarize the attached file"` and confirm the page acknowledges the attachment before submit.
7. After a run that produces an artifact, confirm `gpt artifacts list` shows it and `gpt artifacts pull last --to .` retrieves it.
8. Run `gpt research "investigate launch blockers"` and confirm progress states advance until completion or typed failure.
9. After at least one download, confirm the attached normal-browser tab is still usable by running `gpt capabilities` or another `gpt ask`.
10. Open two ChatGPT tabs, keep the extension active in only one, rerun `gpt browser attach`, and confirm Tablinq returns to the tab with working extension connectivity.

If you want to retain command output or before/after snapshots from a live check, save them under `docs/smoke/`, for example:

- `gpt browser attach > docs/smoke/smoke-attach.txt`
- `gpt browser connection-info > docs/smoke/smoke-connection-info.txt`
- save any before/after JSON snapshots as `docs/smoke/smoke-pre-snapshot.json` and `docs/smoke/smoke-post-snapshot.json`

## Provider checks

1. Call `POST /v1/chat/completions` with `tablinq-instant` and confirm the response includes `request_id`, `tablinq_job_id`, and warnings metadata.
2. Call `POST /v1/chat/completions` with `"stream": true` and confirm SSE finishes cleanly.
3. Call `GET /v1/provider`, configure an OpenClaude preset, and confirm `primary` requests execute while `subagent` requests are rejected with `subagents_disabled`.
4. Upload a file to `POST /v1/files`, run a research completion with the returned file ID, and confirm `GET /v1/artifacts` returns a linked artifact.
5. Run `scripts/soak_gateway.ps1` for repeated sequential requests and confirm request IDs advance without leaving the gateway or daemon unhealthy.

## Interruption and recovery checks

1. Trigger a mid-stream reload and confirm either recovery or a typed timeout or reconciliation failure with a linked failure bundle.
2. Drop the local client connection during a streaming provider request and confirm the request can later be inspected by `request_id`.
3. Confirm an interrupted request never leaves the CLI hanging forever.

## Refusal and auth checks

1. Sign out the attached normal-browser session and confirm provider requests fail with `auth_required`, guidance text, and a `tablinq_failure_bundle_id`.
2. Switch to another account and confirm missing live controls fail as `capability_missing` rather than silently downgrading behavior.

## Managed fallback check

Only run this if attach-first cannot be enabled on the machine you are testing.

1. Set `browser_connection_mode = "managed_only"` in `config/config.toml`.
2. Run `gpt browser open`.
3. Confirm the fallback path works, but keep attach-first as the default smoke path.

## Capture on failure

When something fails, keep:

- `gpt doctor`
- `gpt capabilities`
- `GET /v1/provider`
- the failing CLI command
- the failing gateway request and `request_id`
- the latest bundle under `crash-bundles/`
- a short note describing the visible UI drift or missing control
