# OpenClaude

Use Tablinq's local gateway as an OpenAI-compatible provider.

## Preset values

- Base URL: `http://127.0.0.1:3847/v1`
- API key: `gateway.api_key` from `config.toml`
- Streaming: `true`
- Max parallel requests: `1`
- Subagents: disabled by default
- Branches: at most `1`
- Ask/search timeout: start at `180s`
- Research timeout: start at `900s`

## Recommended models

- `tablinq-instant`
- `tablinq-thinking-standard`
- `tablinq-thinking-extended`
- `tablinq-search`

## Request annotations

- `tablinq_request_role = "primary"` for the main user request
- `tablinq_request_role = "branch"` for serialized branch work
- `tablinq_request_role = "subagent"` for delegated work that should be rejected by default
- `tablinq_parent_request_id = "<request-id>"` when a branch or subagent belongs to a primary request

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

## Session guidance

- For isolated tasks, send `tablinq_new_conversation = true`.
- For follow-up work, reuse the returned `tablinq_conversation_id`.
- Keep `tablinq_request_role = "primary"` unless the client is intentionally creating serialized branch work.

## Cancellation guidance

- Keep streaming on for interactive runs.
- Store the returned `request_id`.
- Cancel with `POST /v1/requests/{request_id}/cancel`.

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

See [Provider mode](provider-mode.md) for bring-up, health checks, files, artifacts, and recovery commands.
