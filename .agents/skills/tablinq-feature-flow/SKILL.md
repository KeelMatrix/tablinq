---
name: tablinq-feature-flow
description: Implement features in the Tablinq repository with minimal cross-layer churn. Use when adding or modifying user-facing behavior and Codex needs to choose the correct entry layer, keep edits local, and validate the smallest meaningful surface.
---

Follow the repository `AGENTS.md` as the baseline policy. This skill only adds feature-implementation workflow.

## Entry selection

Start from the user-facing entrypoint that owns the behavior:

- CLI command or flags: `crates/gpt`
- durable execution, state, recovery, or orchestration: `crates/gptd`
- HTTP API surface: `crates/tablinq-gateway`
- native host behavior: `crates/tablinq-host`
- browser runtime behavior: `packages/browser-adapter`
- page bridge or capability data: `packages/extension`

## Implementation rules

- Keep the first implementation inside one crate or package when possible.
- Add cross-layer changes only when the current layer cannot represent the feature correctly.
- Touch shared protocol or schemas only when the external or cross-process contract actually changes.
- Prefer extending existing tests near the owning layer over introducing a new wide integration path.

## Decision checks

Before editing, confirm:

- what the user-visible behavior should become
- which layer owns that behavior
- whether the change requires a contract change or only local logic
- the narrowest validation loop that can prove the change

## Output

State:
- owning layer chosen
- whether any cross-layer contract changed
- exact validation run
- any deliberate non-goals kept out of scope
