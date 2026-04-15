# Provider Mode

Tablinq can run as a local OpenAI-compatible provider backed by the same attach-first browser session used by the CLI.

This is the right fit when you want tools or agent frameworks to talk to a real ChatGPT browser session through a stable local HTTP surface while keeping the browser as the source of truth.

## Bring-up

1. Build and install Tablinq.
2. If you are using Edge attach-first, load the unpacked extension from the installed `extension` directory into the Edge profile you will attach to.
3. Start the control plane:
   - `gpt daemon start`
   - `gpt gateway start`
4. Start your normal browser in attachable mode:
   - `gpt browser start-debug`
5. If you are using Chrome attach-first, load the unpacked extension into the dedicated persistent Chrome attach profile under the Tablinq state root.
6. Sign in manually at `https://chatgpt.com/` in that browser profile.
7. Attach and verify health:
   - `gpt browser attach`
   - `gpt status`
   - `gpt doctor`
   - `GET /health`
   - `GET /ready`
   - `GET /v1/provider`

This is the intended product flow. The old managed Playwright-owned login path is not the normal startup story anymore because anti-bot protections at `chatgpt.com` made automation-owned login the wrong operational default.

## Core principle

Tablinq is not trying to impersonate a fully parallel cloud API. Safe defaults matter more than synthetic throughput.

Recommended posture:

- keep `max_parallel_requests = 1`
- keep `max_parallel_branches = 1`
- keep subagents disabled by default
- prefer `stream = true` for interactive work and fast cancellation
- set client timeouts above Tablinq's own upstream timeouts

## Core endpoints

- `GET /health`
- `GET /ready`
- `GET /v1/models`
- `GET /v1/capabilities`
- `GET /v1/provider`
- `GET /v1/sessions`
- `GET /v1/usage`
- `GET /v1/requests/{request_id}`
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
- prefer one warm attach-first browser session over synthetic parallel browser work

If clients overlap, queueing is normal. Treat the gateway as a browser-backed scheduler, not a raw stateless endpoint.

## Request annotations

- `tablinq_request_role = "primary"`
- `tablinq_request_role = "branch"`
- `tablinq_request_role = "subagent"`
- `tablinq_parent_request_id = "<request-id>"`

Default intent:

- `primary`
  - normal user-facing work
- `branch`
  - serialized follow-up or side work behind the same upstream slot
- `subagent`
  - rejected by default unless you deliberately relax policy

## Timeouts and client behavior

Recommended starting points:

- ask or search client timeout: `90-180s`
- research client timeout: `5-15m`

Also:

- keep streaming enabled when the client wants fast cancellation
- on `429`, wait for `retry_after_ms` before retrying
- inspect `GET /v1/requests/{request_id}` or `GET /v1/usage` to separate queue delay from upstream execution time
- treat token usage as unknown or provider-specific unless the integration explicitly expects estimates

## Conversations and session reuse

- set `tablinq_new_conversation = true` for isolated tasks, retries, and batch jobs that should not inherit prior context
- pass `tablinq_conversation_id` only when follow-up context matters
- store `tablinq_conversation_id`, `tablinq_session_id`, and `request_id` from responses client-side
- use `GET /v1/sessions` to inspect recent reusable conversations

## Files and artifacts

1. Upload files with `POST /v1/files`.
2. Pass staged IDs through `tablinq_file_ids` or supported file references in chat content.
3. Read artifact metadata through `GET /v1/artifacts`.
4. Download artifact content through `GET /v1/artifacts/{id}/content`.

## Interruption and recovery behavior

Provider clients should expect a browser-backed lifecycle:

- local client disconnects do not necessarily stop upstream browser work
- interrupted requests may later reconcile as `recovered`
- streaming resumes only when Tablinq can safely match the live browser state to the interrupted request
- prompts are not blindly resubmitted by default

Persist `request_id` and inspect request status when a client connection drops unexpectedly.

## Recovery commands

- `gpt status`
- `gpt doctor`
- `gpt browser start-debug`
- `gpt browser attach`
- `gpt browser connection-info`
- `gpt browser open`
- `gpt browser logout`
- `gpt browser switch-account`
- `gpt browser reset-profile`
- `gpt gateway stop`
- `gpt daemon stop`
- `gpt daemon start`
- `gpt gateway start`

See [openclaude.md](openclaude.md) and [paperclip.md](paperclip.md) for client-specific guidance.
