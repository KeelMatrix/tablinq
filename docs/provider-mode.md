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
- `stream = true` is accepted for OpenAI-compatible SSE framing, but Tablinq waits for the finalized browser-backed response and emits final-only chunks; it never streams live partial ChatGPT drafts
- set client timeouts above Tablinq's own upstream timeouts
- expect small built-in browser interaction jitters and a longer pre-submit jitter; they exist to keep browser-driven actions closer to human pacing and reduce provider countermeasures

## Core endpoints

- `GET /health`
- `GET /ready`
- `GET /admin/status`
- `POST /admin/shutdown`
- `POST /admin/clear`
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

`POST /admin/shutdown` and `POST /admin/clear` are administrative operations. `POST /admin/clear` is destructive and requires an authenticated JSON body of `{"all":true,"force":true}`.

## Response size controls

Some state and diagnostics endpoints can return large JSON bodies because they expose durable request, job, conversation, file, artifact, usage, and browser-evidence state. Production clients should request bounded or compact responses for polling and repeated health loops, then opt into full detail only when debugging a specific request.

Common controls:

- `limit=<n>` and `offset=<n>` paginate list-like responses. Use a small limit such as `20` for polling and increase it only for one-off inspection.
- the gateway accepts `limit` values from `1` through `100`
- routes that previously returned unbounded lists still preserve that default when no response-shaping query is supplied; production clients should send `limit` or `view=summary` explicitly
- `view=summary` without an explicit `limit` uses a default page size of `50`
- paginated responses include metadata such as `limit`, `offset`, `returned`, `has_more`, and `next_offset` when another page is available.
- `view=summary` returns compact list/detail shapes suitable for production polling.
- `view=detail` returns the normal diagnostic detail for a route.
- `view=full` keeps full debugging data available.
- `include=<field>` expands heavy optional fields such as `evidence`, `messages`, `snapshot`, `raw`, or `debug` where that route supports them. Comma-separated includes are accepted on routes that expose more than one expansion.
- `compact=true` asks large diagnostic payloads to omit or summarize heavy fields. `compact=false` or `view=full` is the explicit debugging path.
- compacted or bounded responses report metadata such as `compact`, `truncated`, `truncated_fields`, `max_items`, `returned_items`, `has_more`, or route-specific pagination fields.

Invalid response-shaping query parameters return HTTP `400` JSON using the normal nested error envelope. This includes invalid `limit`, negative `offset`, unsupported `view`, unsupported `include`, unsupported route/query combinations, and invalid boolean values for `compact` or `summary`.

Unknown query parameters that do not affect response shape remain compatibility-tolerant and are ignored.

Route guidance:

- `GET /v1/requests`: use `limit`, `offset`, and `view=summary|detail|full`. Production polling should use `view=summary&limit=<n>`. Debugging can use `view=full`.
- `GET /v1/requests/{request_id}`: use `include=evidence,debug,raw` only when the client needs those heavy fields.
- `GET /v1/requests/{request_id}/evidence`: use pagination metadata when evidence is large; this route remains request-scoped debugging data, not a scraping API.
- `GET /v1/jobs`: use `limit`, `offset`, and `view=summary|detail|full`.
- `GET /v1/conversations`: use `limit`, `offset`, and compact summaries for discovery.
- `GET /v1/conversations/{conversation_id}?include=messages`: embeds a bounded first page of messages and includes nested `pagination` metadata with `limit`, `offset`, `returned`, `has_more`, `next_offset`, and `total_items`.
- `GET /v1/conversations/{conversation_id}/messages`: use `limit`, `offset`, and `order=asc|desc` when supported. Avoid unbounded transcript reads in production loops.
- `GET /v1/conversations/{conversation_id}/snapshot`: use `compact=true` for replay planning and `include=messages,artifacts,debug,raw` only for targeted inspection. Compact snapshots include `max_items`, `returned_items`, and `has_more` for bounded `recent_messages`.
- `GET /v1/usage`: use `limit`, `offset`, and `summary=true` for polling, or `summary=false`/`view=full` for detailed history. Pagination metadata is computed against the durable request-history count.
- `GET /v1/files` and `GET /v1/artifacts`: use `limit`, `offset`, and summary views for repeated listing; fetch individual details or artifact content only when needed.

## Supported API presets

- `tablinq-instant`
- `tablinq-thinking-standard`
- `tablinq-thinking-extended`
- `tablinq-search` for Search chip / composer mode
- `tablinq-research` for the Deep Research flow

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
- `tablinq_conversation_id = "<conversation-id>"`
- `tablinq_new_conversation = true`
- `tablinq_auto_recover_provider_generation_failure = true`

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

- use `POST /v1/requests/{request_id}/cancel` for cancellation; `stream = true` only changes the response framing after finalization
- on `429`, wait for `retry_after_ms` before retrying
- use `GET /v1/requests?status=<status>&limit=<n>&offset=<n>&view=summary` for paginated request polling
- inspect `GET /v1/requests/{request_id}` or `GET /v1/usage?summary=true` to separate queue delay from upstream execution time
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
- discover: use `GET /v1/conversations?limit=<n>&offset=<n>` only to list recent reusable conversation IDs and route hints
- inspect metadata: use `GET /v1/conversations/{conversation_id}` for direct conversation detail, counts, latest known request or job IDs, session info, and recovery-manifest metadata
- inspect history: use `GET /v1/conversations/{conversation_id}/messages?limit=<n>&offset=<n>` for normalized agent-facing history with explicit provenance and fallback summaries when exact provider transcript data is unavailable
- reuse safely: use `GET /v1/conversations/{conversation_id}/snapshot?compact=true` when an agent needs the best preserved replay-safe context packet for continuing or replaying work in a fresh conversation
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
  "http://127.0.0.1:3847/v1/conversations/conv-123/snapshot?compact=true"
```

## Files and artifacts

1. Upload files with `POST /v1/files`.
2. Pass staged IDs through `tablinq_file_ids` or supported file references in chat content.
3. Read artifact metadata through `GET /v1/artifacts?limit=<n>&offset=<n>`.
4. Download artifact content through `GET /v1/artifacts/{id}/content`.

`POST /v1/chat/completions` also accepts direct local file paths through `tablinq_files`, but uploaded `tablinq_file_ids` are the safer provider-facing contract. Tool calling, structured response formats other than plain text, and `n > 1` are rejected because the upstream execution is one browser conversation, not a parallel stateless API. `stream = true` returns OpenAI-compatible SSE chunks only after the browser-backed answer finalizes.

## Interruption and recovery behavior

Provider clients should expect a browser-backed lifecycle:

- local client disconnects do not necessarily stop upstream browser work
- interrupted requests may later reconcile as `recovered`
- live token streaming is disabled for correctness because browser-backed ChatGPT drafts are not append-only stable
- OpenAI-compatible `stream = true` responses are final-only SSE: one final content chunk, one stop chunk, then `[DONE]`
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

## Client retry and idempotency guidance

Tablinq is a browser-backed local provider. Clients should use normal HTTP timeouts and may retry idempotent read requests such as `GET /health`, `GET /v1/requests`, `GET /v1/jobs`, `GET /v1/conversations`, and `GET /v1/usage`.

Do not blindly retry prompt-submitting `POST /v1/chat/completions` after a timeout, disconnect, or transport error. The request may already have reached the gateway or browser. Instead, send a stable `X-Request-Id` when possible, persist returned `request_id` values, and inspect:

- `GET /v1/requests/{request_id}`
- `GET /v1/requests/{request_id}/evidence`
- `GET /v1/requests`
- `GET /v1/jobs`

Clients do not need artificial sleeps between normal API calls. If a client sends bursty requests, it should still respect Tablinq’s browser-backed concurrency model and use the documented queue/status endpoints to observe progress.

For prompt POSTs, treat structured HTTP errors as authoritative. Transport-level failures should be considered ambiguous unless request state proves the request was not accepted.
