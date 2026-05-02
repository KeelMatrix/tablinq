---
name: tablinq-skill-selector
description: Automatically select the right Tablinq repo-local and installed engineering skills for a task. Use for almost any Tablinq coding, debugging, design, triage, testing, browser, gateway, daemon, packaging, or architecture request when the user did not explicitly name a skill. Routes to the narrowest relevant skill combination before work begins.
---

# Tablinq Skill Selector

Follow `AGENTS.md` first. This skill only chooses which skill guidance to apply.

## Selection order

Use the first matching path unless the task clearly spans multiple paths.

1. Unclear, broad, architectural, or design-sensitive change:
   - Use `/grill-with-docs` first.
   - Then use `tablinq-feature-flow` or the layer-specific skill.

2. Feature implementation:
   - Use `tablinq-feature-flow`.
   - Use `/tdd` when adding behavior with a testable seam.
   - Use `tablinq-test-selection` before validation.

3. Bug, regression, failing command, runtime error, flaky behavior:
   - Use `/diagnose`.
   - Use `tablinq-bug-triage` for Tablinq layer ownership.
   - Add the relevant layer skill:
     - daemon/state/recovery/IPC: `tablinq-daemon-debug`
     - browser adapter/extension/selectors/capabilities: `tablinq-browser-debug`
     - real signed-in ChatGPT session/live prompt/upload: `tablinq-browser-session-debug`
     - gateway HTTP/OpenAI-compatible API behavior: `tablinq-gateway-surface`
     - failure bundle/log/screenshot/DOM evidence: `tablinq-failure-bundle-review`

4. Validation choice:
   - Use `tablinq-test-selection` whenever deciding what to run after a change.

5. GitHub issue workflow:
   - Use `/triage` for issue triage.
   - Use `/to-prd` when turning an already-discussed plan into a PRD.
   - Use `/to-issues` when breaking a PRD or plan into vertical implementation issues.

6. Codebase understanding:
   - Use `/zoom-out` when the user asks how an area works or Codex is unfamiliar with a path.

7. Architecture improvement:
   - Use `/improve-codebase-architecture` when the user asks for refactoring opportunities, better seams, deeper modules, testability improvements, or AI-navigability improvements.

## Composition rules

- Prefer one repo-local layer skill plus one general engineering skill.
- Do not stack many skills unless the task genuinely crosses boundaries.
- Do not use live browser/session guidance unless live ChatGPT interaction is actually required.
- Do not use issue workflow skills unless the user is working with GitHub Issues or asks for planning artifacts.
- Always state which skills were selected and why in the final response, matching the existing `AGENTS.md` output rule.

## Default routing

If the task is ambiguous but clearly about Tablinq:

- Start with `tablinq-feature-flow` for requested behavior changes.
- Start with `/diagnose` plus `tablinq-bug-triage` for broken behavior.
- Start with `/grill-with-docs` for design changes where the target behavior is not fully specified.
