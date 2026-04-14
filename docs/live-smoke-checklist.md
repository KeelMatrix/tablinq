# Live Smoke Checklist

This checklist stays out of CI. Use it when you want to validate the managed browser flow against the real ChatGPT site with a local profile.

## Preconditions

- install locally with `scripts/install.ps1` or `./scripts/install.sh`
- start the daemon with `gpt daemon start`
- start the gateway with `gpt gateway start`
- open the managed browser with `gpt browser open`
- sign in through the managed profile if required

Confirm `gpt doctor` reports:

- a running daemon
- a detected browser
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
7. After a run that produces an artifact, confirm `gpt artifacts list` shows it and `gpt artifacts pull last` retrieves it.
8. Run `gpt research "investigate launch blockers"` and confirm progress states advance until completion or typed failure.
9. After at least one download, confirm the managed tab is still usable by running `gpt capabilities` or another `gpt ask`.

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

1. Sign out the managed session and confirm provider requests fail with `auth_required`, guidance text, and a `tablinq_failure_bundle_id`.
2. Switch to another account and confirm missing live controls fail as `capability_missing` rather than silently downgrading behavior.

## Capture on failure

When something fails, keep:

- `gpt doctor`
- `gpt capabilities`
- `GET /v1/provider`
- the failing CLI command
- the failing gateway request and `request_id`
- the latest bundle under `crash-bundles/`
- a short note describing the visible UI drift or missing control
