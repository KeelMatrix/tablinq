# Provider Mode

Tablinq can run as a local OpenAI-compatible provider backed by the same attach-first browser session used by the CLI.

This is the right fit when you want tools or agent frameworks to talk to a real ChatGPT browser session through a stable local HTTP surface while keeping the browser as the source of truth.

## Bring-up

1. Build and install Tablinq.
2. If you are using Edge attach-first, load the unpacked extension from the installed `extension` directory into the Edge profile you will attach to.
3. Start the control plane:
   - `gpt daemon start`
   - `gpt gateway start`
4. Start an attachable browser session:
   - `gpt browser start-debug`
5. If you are using Chrome attach-first, load the unpacked extension into the dedicated persistent Chrome attach profile under the Tablinq state root.
6. Sign in manually at `https://chatgpt.com/` in that same attach target profile.
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
- expect small built-in browser interaction jitters and a longer pre-submit jitter; they exist to keep browser-driven actions closer to human pacing and reduce provider countermeasures

## Core endpoints

- `GET /health`
- `GET /ready`
- `GET /v1/models`
- `GET /v1/capabilities`
- `GET /v1/provider`
- `GET /v1/sessions`
- `GET /v1/usage`
- `GET /v1/requests`
- `GET /v1/requests/{request_id}`
- `GET /v1/requests/{request_id}/evidence`
- `GET /v1/conversations`
- `GET /v1/conversations/{conversation_id}`
- `GET /v1/conversations/{conversation_id}/messages`
- `GET /v1/conversations/{conversation_id}/snapshot`
- `POST /v1/conversations/select`
- `GET /v1/sessions/{session_id}/conversations`
- `POST /v1/chat/completions`
- `POST /v1/files`
- `GET /v1/files`
- `GET /v1/files/{id}`
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
- use `GET /v1/requests?status=<status>&limit=<n>&offset=<n>` for paginated request polling
- inspect `GET /v1/requests/{request_id}` or `GET /v1/usage` to separate queue delay from upstream execution time
- inspect `GET /v1/requests/{request_id}/evidence` when a request times out, recovers, or looks uncertain; Tablinq captures a compact browser-state artifact automatically for those cases
- treat token usage as unknown or provider-specific unless the integration explicitly expects estimates

## Evidence workflow

- persist `request_id` from every `POST /v1/chat/completions` response
- poll `GET /v1/requests/{request_id}` to distinguish queue delay, failure, timeout, cancellation, and `recovered` state
- fetch `GET /v1/requests/{request_id}/evidence` when a request times out, recovers, or otherwise looks uncertain
- treat the returned artifact as a compact browser-state snapshot for agent analysis: current URL, timestamp, full-page screenshot path, and cleaned DOM text with scripts and styles removed
- do not treat `/evidence` as a general scraping API; it exists only for request-scoped debugging and recovery analysis

## Conversations and session reuse

- create: send `POST /v1/chat/completions` with `tablinq_new_conversation = true` when the next task must not inherit browser context
- persist: store `tablinq_conversation_id`, `tablinq_session_id`, and `request_id` from chat responses client-side
- discover: use `GET /v1/conversations` only to list recent reusable conversation IDs and route hints
- inspect metadata: use `GET /v1/conversations/{conversation_id}` for direct conversation detail, counts, latest known request or job IDs, session info, and recovery-manifest metadata
- inspect history: use `GET /v1/conversations/{conversation_id}/messages?limit=<n>&offset=<n>` for normalized agent-facing history with explicit provenance and fallback summaries when exact provider transcript data is unavailable
- reuse safely: use `GET /v1/conversations/{conversation_id}/snapshot` when an agent needs the best preserved replay-safe context packet for continuing or replaying work in a fresh conversation
- select active browser thread: call `POST /v1/conversations/select` only when you need the browser to switch to an existing thread or clear the active selection; do not use it for history retrieval
- reuse in-place: pass the stored `tablinq_conversation_id` back to `POST /v1/chat/completions` only when follow-up context should stay in that browser conversation
- session scope: use `GET /v1/sessions/{session_id}/conversations` when you need conversations for one attached browser session rather than the global list

Small examples:

```bash
curl -H "Authorization: Bearer $TABLINQ_API_KEY" \
  http://127.0.0.1:3847/v1/conversations/conv-123

curl -H "Authorization: Bearer $TABLINQ_API_KEY" \
  "http://127.0.0.1:3847/v1/conversations/conv-123/messages?limit=20&offset=0"

curl -H "Authorization: Bearer $TABLINQ_API_KEY" \
  http://127.0.0.1:3847/v1/conversations/conv-123/snapshot
```

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

For the provider page failure `Something went wrong while generating the response...`, Tablinq uses a separate recovery path:

- classify the failure as `provider_generation_failed_nonretryable`
- do not auto-click the page Retry button
- reload the tab
- start a fresh conversation
- replay once with a compact preserved-context wrapper

The replay wrapper currently includes:

- a short recovery note
- a compact context summary derived from the prior durable recovery manifest
- the original user request

The current durable recovery manifest is richer than a single-turn snapshot. It is refreshed after each completed turn and stores archived context, recent turn summaries, tracked input attachments, and tracked generated artifacts. On provider-generation recovery, Tablinq may also upload a generated recovery-manifest attachment and a bounded set of recent durable context files into the fresh conversation.

This is still not a full raw chat dump. If clients need exact transcript reconstruction or provider-only thread state, they should still persist their own higher-fidelity context in addition to `request_id` and conversation ids.

For agent inspection, Tablinq now exposes the same durable state through `GET /v1/conversations/{conversation_id}/messages` and `GET /v1/conversations/{conversation_id}/snapshot`. Those endpoints are intentionally honest about provenance:

- exact request or job text is surfaced as exact request-detail history
- preserved manifest turns are surfaced as recovery-manifest history
- older summarized context stays summarized instead of being promoted to a fake exact transcript

Persist `request_id` and inspect request status when a client connection drops unexpectedly.

## Recovery commands

- `gpt status`
- `gpt doctor`
- `gpt browser start-debug`
- `gpt browser attach`
- `gpt browser connection-info`
- `gpt browser open`
- `gpt gateway stop`
- `gpt daemon stop`
- `gpt daemon start`
- `gpt gateway start`

See [openclaude.md](openclaude.md) and [paperclip.md](paperclip.md) for client-specific guidance.
