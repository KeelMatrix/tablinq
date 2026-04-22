---
name: tablinq-browser-debug
description: Debug attach-first browser issues in the Tablinq repository. Use when work centers on `packages/browser-adapter`, `packages/extension`, browser attach and detach behavior, ChatGPT tab discovery, capability probes, screenshots, uploads, downloads, or page-driven failures.
---

Follow the repository `AGENTS.md` as the baseline policy. This skill only adds browser-path debugging guidance.

## Scope the failure first

Decide which browser-side boundary is failing:

- debug endpoint discovery or attach
- target-tab discovery or focus
- extension connection or capability publication
- selector or alias mismatch
- upload, download, screenshot, or page action failure
- disconnect, reload, or interruption recovery

## Inspection order

1. `packages/browser-adapter/src`
2. Matching browser-adapter tests
3. `packages/extension/src` and matching extension tests if the symptom depends on capabilities, aliases, or page probes
4. Selector data only if the failure is clearly selector-specific

## Working rules

- Preserve the attach-first model unless the task explicitly requires changing fallback behavior.
- Separate transport failures from semantic page failures before editing code.
- Treat the extension as the semantic source of truth for capabilities and aliases.
- Prefer evidence from logs, screenshots, and failure bundles over guesswork.
- Use Playwright only as an attach-and-assist runtime, not as a reason to redesign ownership boundaries.

## Validation

- Prefer the narrowest affected test file first.
- If live browser validation is required, use attach-first commands and state the manual prerequisite when sign-in or browser state is needed.
- Do not touch fake harness code unless the task is actually a harness issue or test gap.

## Output

State:
- failing browser boundary
- owning package
- whether the root cause is transport, extension, selector, or provider-page behavior
- exact validation performed
