# OpenClaude

Use Tablinq's local gateway as an OpenAI-compatible provider for OpenClaude.

## Good default posture

- base URL: `http://127.0.0.1:3847/v1`
- API key: `gateway.api_key` from `config.toml`
- streaming: `true`
- max parallel requests: `1`
- max parallel branches: `1`
- subagents: disabled by default
- ask/search timeout: start around `180s`
- research timeout: start around `900s`

OpenClaude works best when you treat Tablinq as a high-context, browser-backed provider with one true upstream slot.

## Recommended models

- `tablinq-instant`
- `tablinq-thinking-standard`
- `tablinq-thinking-extended`
- `tablinq-search`

Use `tablinq-research` only when you explicitly want the longer-running research path.

## Safe default policy

- primary: execute
- branch: serialize
- subagent: reject with `subagents_disabled`

## Recommended config

```json
{
  "provider": {
    "baseURL": "http://127.0.0.1:3847/v1",
    "apiKey": "<gateway.api_key>"
  },
  "model": "tablinq-search",
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

## Cancellation and recovery guidance

- keep streaming enabled for interactive sessions
- persist the returned `request_id`
- cancel with `POST /v1/requests/{request_id}/cancel`
- if the client disconnects, inspect `GET /v1/requests/{request_id}` before assuming the work is lost

Because Tablinq is browser-backed, a local disconnect may become a later `recovered` result instead of a simple failure.

## Example request

```json
{
  "model": "tablinq-search",
  "stream": true,
  "tablinq_request_role": "primary",
  "tablinq_new_conversation": true,
  "messages": [
    { "role": "user", "content": "Summarize the latest release notes." }
  ]
}
```

See [provider-mode.md](provider-mode.md) for bring-up, health checks, files, artifacts, and recovery commands.
