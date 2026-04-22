---
name: tablinq-bug-triage
description: Triage bugs in the Tablinq repository. Use when a user reports a failing command, runtime error, regression, or unclear breakage and Codex needs to identify the owning layer, inspect the minimum relevant files, and choose the narrowest useful validation path.
---

Follow the repository `AGENTS.md` as the baseline policy. This skill only adds the bug-triage workflow.

## Triage flow

1. Start from the failing surface the user named, not from broad repo search.
2. Classify the bug into one primary owner:
   - CLI or command parsing: `crates/gpt`
   - Daemon, persistence, recovery, browser orchestration: `crates/gptd`
   - OpenAI-compatible HTTP behavior: `crates/tablinq-gateway`
   - Native messaging host: `crates/tablinq-host`
   - Browser attach/runtime behavior: `packages/browser-adapter`
   - Page bridge or capability probes: `packages/extension`
3. Read the owning entrypoint first, then the nearest matching tests.
4. Expand to adjacent layers only when the current owner cannot explain the symptom.

## Debugging heuristics

- For command failures, start at the exact CLI command implementation before inspecting daemon internals.
- For browser failures, distinguish:
  - attach or tab selection problems
  - adapter action failures
  - extension capability or alias mismatches
  - provider page failures
- For recovery issues, trace the persisted request state and recovery-specific code before changing prompt or browser behavior.
- For gateway mismatches, compare the gateway handler contract against the daemon request it emits before changing shared types.

## Validation

- Run the smallest test or command that exercises the suspected owner.
- Escalate from file-level or package-level validation only if the first loop leaves uncertainty.
- If the issue is not covered by tests, state that explicitly and use the narrowest live command that demonstrates the failure.

## Output

Report:

- owning layer
- likely cause
- exact files inspected or changed
- exact validation run
- remaining uncertainty
