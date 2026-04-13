# Paperclip

Use Tablinq's local gateway as an OpenAI-compatible provider for Paperclip.

## Connection values

- Base URL: `http://127.0.0.1:3847/v1`
- API key: `gateway.api_key` from `config.toml`
- Streaming: `true`
- Max parallel requests: `1`

## Recommended models

- `tablinq-instant`
- `tablinq-search`
- `tablinq-research`

## File and artifact flow

1. Upload local files to `POST /v1/files`.
2. Pass returned IDs through `tablinq_file_ids` on `POST /v1/chat/completions`.
3. Read generated artifact metadata from `GET /v1/artifacts`.
4. Download artifact content from `GET /v1/artifacts/{id}/content`.

## Usage guidance

- treat usage accounting as provider-specific or estimated
- keep concurrency at `1`
- respect `retry_after_ms` on `429` responses
- prefer one warm browser session instead of parallel browser work

## Example request

```json
{
  "model": "tablinq-research",
  "stream": false,
  "tablinq_file_ids": ["file-123"],
  "messages": [
    {
      "role": "user",
      "content": "Summarize the uploaded source and produce a report."
    }
  ]
}
```

See [Provider mode](provider-mode.md) for bring-up, health checks, and recovery commands.
