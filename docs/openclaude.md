# OpenClaude

Use Tablinq's local gateway as an OpenAI-compatible provider for OpenClaude.

## Recommended operating posture

- bring up Tablinq through the normal attach-first flow
- base URL: `http://127.0.0.1:3847/v1`
- API key: `gateway.api_key` from `config.toml`
- streaming: omit the option or set `false`
- max parallel requests: `1`
- max parallel branches: `1`
- subagents: disabled by default
- ask or search timeout: start around `180s`
- research timeout: start around `900s`

OpenClaude works best when you treat Tablinq as a high-context, browser-backed provider with one real upstream slot.

## Recommended API presets

- `tablinq-instant`
- `tablinq-thinking-standard`
- `tablinq-thinking-extended`
- `tablinq-search` for the Search composer mode

Use `tablinq-research` only when you explicitly want the longer-running research path.

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
  "model": "tablinq-search",
  "stream": false,
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
- use `GET /v1/conversations/{conversation_id}/messages` for agent-facing history inspection
- use `GET /v1/conversations/{conversation_id}/snapshot` when OpenClaude needs replay-safe continuation context for a fresh conversation
- treat `GET /v1/conversations` as discovery only, and `POST /v1/conversations/select` as browser-thread switching only

## Cancellation and recovery guidance

- keep `stream` omitted or explicitly `false`; browser-backed ChatGPT drafts may be rewritten before finalization, so live streaming is disabled for correctness
- persist the returned `request_id`
- cancel with `POST /v1/requests/{request_id}/cancel`
- if the client disconnects, inspect `GET /v1/requests/{request_id}` before assuming the work is lost
- if the next step needs context reuse rather than status only, fetch `GET /v1/conversations/{conversation_id}/snapshot` before replaying or resuming work

Because Tablinq is browser-backed, a local disconnect may later show up as `recovered` instead of a simple failure.

For request-scoped evidence capture and how to consume `GET /v1/requests/{request_id}/evidence`, use the centralized guidance in [provider-mode.md#evidence-workflow](provider-mode.md#evidence-workflow).

## Example request

```json
{
  "model": "tablinq-search",
  "stream": false,
  "tablinq_request_role": "primary",
  "tablinq_new_conversation": true,
  "messages": [
    { "role": "user", "content": "Summarize the latest release notes." }
  ]
}
```

See [provider-mode.md](provider-mode.md) for bring-up, health checks, files, artifacts, and recovery commands.
