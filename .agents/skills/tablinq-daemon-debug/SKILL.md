---
name: tablinq-daemon-debug
description: Debug daemon-owned behavior in the Tablinq repository. Use when a task centers on `crates/gptd`, local IPC, request lifecycle state, persistence, recovery, supervised browser orchestration, state-root behavior, or durable job execution.
---

Follow the repository `AGENTS.md` as the baseline policy. This skill only adds daemon-specific debugging flow.

## Start from the daemon contract

1. Identify the inbound surface:
   - CLI request
   - gateway request
   - recovered persisted job
2. Trace the request into `crates/gptd`.
3. Locate the specific lifecycle edge:
   - queueing
   - startup
   - running state
   - interruption
   - recovery
   - terminal completion or failure

## Focus areas

- Persisted state transitions must stay explicit and conservative.
- Browser-backed recovery should be inspected before changing prompt replay behavior.
- Distinguish IPC problems from daemon core logic and from browser-adapter subprocess issues.
- Check whether the bug is caused by state-root layout, SQLite persistence, or process supervision before widening scope.

## Validation

- Prefer `cargo test -p gptd` or the narrowest matching daemon tests.
- If the failure is integration-shaped, use the narrowest CLI command that reaches the daemon path under suspicion.
- Do not broaden into gateway or browser-adapter changes unless the daemon evidence requires it.

## Output

Report:
- lifecycle stage affected
- suspected daemon subsystem
- persistence or recovery implications
- exact validation run
