---
name: tablinq-gateway-surface
description: Work on the Tablinq OpenAI-compatible gateway surface. Use when a task centers on `crates/tablinq-gateway`, request and response mapping, files or artifacts endpoints, status or cancellation endpoints, model metadata, or gateway-specific behavior exposed over HTTP.
---

Follow the repository `AGENTS.md` as the baseline policy. This skill only adds gateway-specific workflow.

## Gateway workflow

1. Start at the HTTP endpoint or payload shape the user named.
2. Trace how the gateway maps that request into daemon/core behavior.
3. Determine whether the issue is:
   - HTTP contract behavior
   - gateway request translation
   - daemon response translation
   - concurrency or queueing behavior
   - gateway-only validation or auth behavior

## Working rules

- Preserve OpenAI-compatible behavior where the gateway already intentionally models it.
- Keep gateway fixes in the gateway unless the daemon contract is actually wrong or incomplete.
- Change shared protocol only if the gateway cannot express the required behavior without it.
- Prefer matching gateway module filters such as `tests::chat_requests`, `tests::conversation_inspection`, `tests::failure_recovery_status`, `tests::scheduler_process_control`, or `tests::uploads_artifacts` before broader validation.

## Output

State:

- endpoint or contract under change
- whether the fix stayed gateway-local
- exact validation run
