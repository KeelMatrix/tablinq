# Provider Mode

Tablinq can run as a local OpenAI-compatible provider backed by the same managed browser session used by the CLI. The safe default is one warm browser session, one active upstream request, and no subagents.

## Bring-up

1. Build and install Tablinq.
2. Load the unpacked extension from the installed `extension` directory.
3. Start the control plane:
   - `gpt daemon start`
   - `gpt gateway start`
4. Open the managed browser with `gpt browser open`.
5. Sign in once in the managed profile.
6. Verify health:
   - `gpt status`
   - `gpt doctor`
   - `GET /health`
   - `GET /ready`
   - `GET /v1/provider`

## Core endpoints

- `GET /health`
- `GET /ready`
- `GET /v1/models`
- `GET /v1/capabilities`
- `GET /v1/provider`
- `POST /v1/chat/completions`
- `POST /v1/files`
- `GET /v1/files`
- `GET /v1/artifacts`
- `GET /v1/artifacts/{id}`
- `GET /v1/artifacts/{id}/content`
- `POST /v1/requests/{request_id}/cancel`

## Recommended models

- `tablinq-instant`
- `tablinq-thinking-standard`
- `tablinq-thinking-extended`
- `tablinq-search`
- `tablinq-research`

## Safe parallelism

- keep `max_parallel_requests = 1`
- keep `max_parallel_branches = 1`
- keep subagents disabled by default
- respect `retry_after_ms` on `429` responses
- prefer one warm browser session over multiple managed profiles

## Request annotations

- `tablinq_request_role = "primary"`
- `tablinq_request_role = "branch"`
- `tablinq_request_role = "subagent"`
- `tablinq_parent_request_id = "<request-id>"`

`subagent` requests should be rejected by default. `branch` requests should be serialized through the same queue as primary requests.

## Files and artifacts

1. Upload files through `POST /v1/files`.
2. Pass staged IDs through `tablinq_file_ids` or inline file references in chat content.
3. Read generated artifact metadata from `GET /v1/artifacts`.
4. Download artifact content from `GET /v1/artifacts/{id}/content`.

## Recovery

- `gpt status` or `gpt doctor`
- `gpt browser open`
- `gpt browser logout`
- `gpt browser switch-account`
- `gpt browser reset-profile`
- `gpt gateway stop`
- `gpt daemon stop`
- `gpt daemon start`
- `gpt gateway start`

For client-specific setup, see [OpenClaude](openclaude.md) and [Paperclip](paperclip.md).
