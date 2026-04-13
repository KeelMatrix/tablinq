# OpenClaude

Use Tablinq's local gateway as an OpenAI-compatible provider.

## Preset values

- Base URL: `http://127.0.0.1:3847/v1`
- API key: `gateway.api_key` from `config.toml`
- Streaming: `true`
- Max parallel requests: `1`
- Subagents: disabled by default

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

## Example request

```json
{
  "model": "tablinq-search",
  "stream": true,
  "tablinq_request_role": "primary",
  "messages": [
    { "role": "user", "content": "Summarize the latest release notes." }
  ]
}
```

See [Provider mode](provider-mode.md) for bring-up, health checks, files, artifacts, and recovery commands.
