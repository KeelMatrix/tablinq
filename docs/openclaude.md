# OpenClaude

Use Tablinq's local gateway as an OpenAI-compatible provider for OpenClaude.

## Recommended operating posture

- bring up Tablinq through the normal attach-first flow
- base URL: `http://127.0.0.1:3847/v1`
- API key: `gateway.api_key` from `config.toml`
- streaming: `true` is supported as final-only OpenAI SSE for OpenClaude compatibility; Tablinq does not emit live partial browser text
- max parallel requests: `1`
- max parallel branches: `1`
- subagents: disabled by default
- ask or search timeout: start around `180s`
- research timeout: start around `900s`

OpenClaude works best when you treat Tablinq as a high-context, browser-backed provider with one real upstream slot.

## Recommended API models and modes

- `tablinq-instant`
- `tablinq-thinking-standard`
- `tablinq-thinking-extended`

Search is a ChatGPT composer chip/mode, not a model. Use a base `model` plus top-level `tablinq_mode: "search"` when OpenClaude should ask with Search enabled. Omit `tablinq_mode` or set `"normal"` for non-search requests. Search combines with any selected base target where the attached ChatGPT UI supports that target; Free/Paid plan differences affect base target availability, not the Search chip concept.

Use `tablinq-research` only when you explicitly want the longer-running research path. Research is separate from Search.

## Safe default policy

- primary: execute
- branch: serialize
- subagent: reject with `subagents_disabled`

## Example config

```json
{
  "provider": {
    "baseURL": "http://127.0.0.1:3847/v1",
    "apiKey": "<gateway.api_key>"
  },
  "model": "tablinq-thinking-standard",
  "tablinq_mode": "search",
  "stream": true,
  "max_parallel_requests": 1,
  "max_parallel_branches": 1,
  "subagents": false,
  "request_timeout_ms": 180000
}
```

## Request annotations

- `tablinq_request_role = "primary"` for the main user request
- `tablinq_request_role = "branch"` for serialized follow-up work
- `tablinq_request_role = "subagent"` for delegated work that should be rejected by default
- `tablinq_parent_request_id = "<request-id>"` when branch work belongs to a primary request

## Conversation guidance

- send `tablinq_new_conversation = true` for isolated work
- reuse the returned `tablinq_conversation_id` only for intentional follow-ups
- keep `tablinq_request_role = "primary"` unless the client deliberately creates serialized branch work
- use `GET /v1/conversations/{conversation_id}` for direct conversation metadata instead of scraping `GET /v1/conversations`
- use `GET /v1/conversations/{conversation_id}/messages?limit=<n>&offset=<n>` for agent-facing history inspection
- use `GET /v1/conversations/{conversation_id}/snapshot?compact=true` when OpenClaude needs replay-safe continuation context for a fresh conversation
- treat `GET /v1/conversations?limit=<n>&offset=<n>` as discovery only, and `POST /v1/conversations/select` as browser-thread switching only
- use `view=summary` for request/job polling and `view=full` only while debugging a specific failed request

## Cancellation and recovery guidance

- `stream = true` is accepted, but Tablinq waits for ChatGPT finalization before emitting the final content chunk, a stop chunk, and `[DONE]`
- browser-backed ChatGPT drafts may be rewritten before finalization, so live token streaming remains disabled for correctness
- persist the returned `request_id`
- cancel with `POST /v1/requests/{request_id}/cancel`
- if the client disconnects, inspect `GET /v1/requests/{request_id}` before assuming the work is lost
- if the next step needs context reuse rather than status only, fetch `GET /v1/conversations/{conversation_id}/snapshot?compact=true` before replaying or resuming work

Because Tablinq is browser-backed, a local disconnect may later show up as `recovered` instead of a simple failure.

For request-scoped evidence capture and how to consume `GET /v1/requests/{request_id}/evidence`, use the centralized guidance in [provider-mode.md#evidence-workflow](provider-mode.md#evidence-workflow).

## Example request

```json
{
  "model": "tablinq-thinking-standard",
  "tablinq_mode": "search",
  "stream": true,
  "tablinq_request_role": "primary",
  "tablinq_new_conversation": true,
  "messages": [
    { "role": "user", "content": "Summarize the latest release notes." }
  ]
}
```

See [provider-mode.md](provider-mode.md) for bring-up, health checks, files, artifacts, and recovery commands.
