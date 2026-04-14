# Paperclip

Use Tablinq's local gateway as an OpenAI-compatible provider for Paperclip.

## Connection values

- Base URL: `http://127.0.0.1:3847/v1`
- API key: `gateway.api_key` from `config.toml`
- Streaming: `true`
- Max parallel requests: `1`
- Ask/search timeout: start at `180s`
- Research timeout: start at `900s`

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
- disable or minimize subagents

## Recommended config

```json
{
  "provider": "openai-compatible",
  "baseURL": "http://127.0.0.1:3847/v1",
  "apiKey": "<gateway.api_key>",
  "model": "tablinq-research",
  "stream": true,
  "max_parallel_requests": 1,
  "subagents": false,
  "request_timeout_ms": 900000
}
```

## Session guidance

- Set `tablinq_new_conversation = true` for isolated report jobs.
- Reuse `tablinq_conversation_id` only when a later request should keep the same context.
- Read `tablinq_session_id` and `tablinq_conversation_id` from responses and persist them client-side.

## Cancellation and file reuse guidance

- Store `request_id` and cancel through `POST /v1/requests/{request_id}/cancel`.
- Reuse prior uploads with `tablinq_file_ids` instead of re-uploading unchanged files.
- Use `GET /v1/files` and `GET /v1/artifacts` to inspect reusable assets and outputs.

## Example request

```json
{
  "model": "tablinq-research",
  "stream": true,
  "tablinq_new_conversation": true,
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
