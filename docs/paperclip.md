# Paperclip Compatibility

Tablinq uses the HTTP gateway as the primary Paperclip integration path.

Direct path:

- Paperclip -> Tablinq gateway
- Base URL: `http://127.0.0.1:3847/v1`
- API key: the local bearer token from `config/config.toml` under `gateway.api_key`
- Transport: OpenAI-compatible HTTP
- Streaming: `true`
- Max parallel requests: `1`

Recommended model IDs:

- `tablinq-instant`
- `tablinq-search`
- `tablinq-research`

File and artifact flow:

1. Upload local files to `POST /v1/files` with `x-file-name`.
2. Pass returned file IDs in `tablinq_file_ids` on `POST /v1/chat/completions`.
3. Read generated artifact metadata from `GET /v1/artifacts`.
4. Download artifact content from `GET /v1/artifacts/{id}/content`.

Usage and budget guidance:

- Treat usage as unknown or estimated.
- Keep provider concurrency at `1`.
- Respect `retry_after_ms` on `429` responses.
- Prefer one warm browser session instead of parallel browser work.

Dual-use options:

- Paperclip -> Tablinq for direct local provider access
- Paperclip -> OpenClaude -> Tablinq if you want OpenClaude policy behavior in front of the gateway

Example request body:

```json
{
  "model": "tablinq-research",
  "stream": false,
  "tablinq_file_ids": ["file-123"],
  "messages": [
    { "role": "user", "content": "Summarize the uploaded source and produce a report." }
  ]
}
```
