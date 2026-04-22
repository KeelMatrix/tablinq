---
name: tablinq-failure-bundle-review
description: Review Tablinq failure bundles and related logs. Use when a task involves crash bundles, screenshots, DOM snapshots, ARIA snapshots, selector attempts, active URL captures, or linked logs and Codex needs to infer the most likely root cause before changing code.
---

Follow the repository `AGENTS.md` as the baseline policy. This skill only adds failure-bundle review workflow.

## Review order

1. Identify the failing request and the reported symptom.
2. Read the bundle metadata first.
3. Inspect logs and selector attempts before opening broad source files.
4. Use screenshots or DOM evidence to decide whether the failure is:
   - page-state mismatch
   - selector mismatch
   - capability publication issue
   - provider-side failure
   - local orchestration or timeout issue

## Interpretation rules

- Prefer evidence in the bundle over assumptions from code structure.
- Separate provider-page failures from local automation failures before editing recovery logic.
- If the bundle points to a selector-specific issue, only then inspect selector-pack or alias-related code.
- If the bundle lacks enough evidence, say what is missing instead of overcommitting to a cause.

## Output

Summarize:

- likely failure class
- strongest evidence from the bundle
- most likely owning layer
- whether code changes are justified or more evidence is needed
