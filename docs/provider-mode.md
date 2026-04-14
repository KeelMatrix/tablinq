# Provider Mode

Tablinq can run as a local OpenAI-compatible provider backed by the same managed browser session used by the CLI. The safe default is one warm browser session, one active upstream request, and no subagents.

## Recommended settings

- keep `max_parallel_requests = 1`
- keep `max_parallel_branches = 1`
- keep subagents disabled or minimized
- prefer `stream = true` for interactive work and cancellation responsiveness
- use a client timeout above the gateway request timeout, typically `90-180s` for ask/search and `5-15m` for research
- reuse `tablinq_conversation_id` only when follow-up context matters; otherwise set `tablinq_new_conversation = true`
- treat token usage as unknown and tune from `duration_ms` and `queue_wait_ms` instead

## Bring-up

1. Build and install Tablinq.
2. Load the unpacked extension from the installed `extension` directory.
3. Start the control plane:
   - `gpt daemon start`
   - `gpt gateway start`
4. Open the managed browser with `gpt browser open`.
5. Sign in once in the managed profile.
6. Verify health:
   - `gpt status`
   - `gpt doctor`
   - `GET /health`
   - `GET /ready`
   - `GET /v1/provider`

## Core endpoints

- `GET /health`
- `GET /ready`
- `GET /v1/models`
- `GET /v1/capabilities`
- `GET /v1/provider`
- `POST /v1/chat/completions`
- `POST /v1/files`
- `GET /v1/files`
- `GET /v1/artifacts`
- `GET /v1/artifacts/{id}`
- `GET /v1/artifacts/{id}/content`
- `POST /v1/requests/{request_id}/cancel`

## Recommended models

- `tablinq-instant`
- `tablinq-thinking-standard`
- `tablinq-thinking-extended`
- `tablinq-search`
- `tablinq-research`

## Safe parallelism

- keep `max_parallel_requests = 1`
- keep `max_parallel_branches = 1`
- keep subagents disabled by default
- respect `retry_after_ms` on `429` responses
- prefer one warm browser session over multiple managed profiles

## Request annotations

- `tablinq_request_role = "primary"`
- `tablinq_request_role = "branch"`
- `tablinq_request_role = "subagent"`
- `tablinq_parent_request_id = "<request-id>"`

`subagent` requests should be rejected by default. `branch` requests should be serialized through the same queue as primary requests.

## Timeout and cancellation guidance

- expect a single browser-backed upstream slot; queueing is normal when clients overlap
- on `429`, wait for `retry_after_ms` before retrying
- cancel work through `POST /v1/requests/{request_id}/cancel`
- keep streaming enabled when the client wants fast stop behavior
- inspect `GET /v1/requests/{request_id}` or `GET /v1/usage` to separate queue delay from upstream execution time

## Session reuse guidance

- set `tablinq_new_conversation = true` for isolated tasks, batch jobs, and retries that should not inherit prior context
- pass `tablinq_conversation_id` to continue an existing conversation explicitly
- read `tablinq_conversation_id` and `tablinq_session_id` from chat responses and store them client-side
- use `GET /v1/sessions` to inspect recent reusable conversations before continuing work

## Files and artifacts

1. Upload files through `POST /v1/files`.
2. Pass staged IDs through `tablinq_file_ids` or inline file references in chat content.
3. Read generated artifact metadata from `GET /v1/artifacts`.
4. Download artifact content from `GET /v1/artifacts/{id}/content`.

File responses include stable IDs, file metadata, and linked request or conversation IDs. Artifact responses include stable IDs, content URLs, linked job IDs, and linked request IDs.

## Recovery

- `gpt status` or `gpt doctor`
- `gpt browser open`
- `gpt browser logout`
- `gpt browser switch-account`
- `gpt browser reset-profile`
- `gpt gateway stop`
- `gpt daemon stop`
- `gpt daemon start`
- `gpt gateway start`

For client-specific setup, see [OpenClaude](openclaude.md) and [Paperclip](paperclip.md).
