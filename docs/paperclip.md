# Paperclip

Use Tablinq's local gateway as an OpenAI-compatible provider for Paperclip.

## Recommended operating posture

- bring up Tablinq through the normal attach-first flow
- base URL: `http://127.0.0.1:3847/v1`
- API key: `gateway.api_key` from `config.toml`
- streaming: `true`
- max parallel requests: `1`
- ask or search timeout: start around `180s`
- research timeout: start around `900s`
- subagents: disabled or minimized

Paperclip is a good fit when you want file-backed browser automation, research-style runs, and artifact retrieval through a familiar provider contract.

## Recommended models

- `tablinq-instant`
- `tablinq-search`
- `tablinq-research`

## Example config

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

## File and artifact flow

1. Upload local files to `POST /v1/files`.
2. Pass returned IDs through `tablinq_file_ids` on `POST /v1/chat/completions`.
3. Read generated artifact metadata from `GET /v1/artifacts`.
4. Download artifact content from `GET /v1/artifacts/{id}/content`.

## Usage guidance

- keep concurrency at `1`
- respect `retry_after_ms` on `429` responses
- prefer one warm browser session over parallel browser work
- treat usage accounting as provider-specific or estimated
- reuse prior uploads with `tablinq_file_ids` instead of re-uploading unchanged files

## Conversation guidance

- set `tablinq_new_conversation = true` for isolated report or batch jobs
- reuse `tablinq_conversation_id` only when later work should inherit that context
- persist `request_id`, `tablinq_session_id`, and `tablinq_conversation_id` from responses client-side

## Cancellation and recovery guidance

- store `request_id`
- cancel through `POST /v1/requests/{request_id}/cancel`
- if the client drops mid-run, inspect request status before retrying
- do not assume a disconnected research job is lost; it may complete upstream and reconcile as `recovered`

For request-scoped evidence capture and how to consume `GET /v1/requests/{request_id}/evidence`, use the centralized guidance in [provider-mode.md#evidence-workflow](provider-mode.md#evidence-workflow).

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

See [provider-mode.md](provider-mode.md) for bring-up, health checks, and recovery commands.
