# OpenClaude Compatibility

Tablinq can be used as a local OpenAI-compatible provider through the gateway.

Use these values for an OpenClaude preset:

- Base URL: `http://127.0.0.1:3847/v1`
- API key: the local bearer token from `config/config.toml` under `gateway.api_key`
- Streaming: `true`
- Max parallel requests: `1`
- Subagents: disabled by default

Recommended model IDs:

- `tablinq-instant`
- `tablinq-thinking-standard`
- `tablinq-search`

Request annotations:

- `tablinq_request_role = "primary"` for the main user request
- `tablinq_request_role = "branch"` for serialized branch work
- `tablinq_request_role = "subagent"` for delegated work that should be rejected by default
- `tablinq_parent_request_id = "<request-id>"` when a branch or subagent belongs to a primary request

Safe default policy:

- primary: execute
- branch: serialize through the single browser session
- subagent: reject with `subagents_disabled`

Example request body:

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
