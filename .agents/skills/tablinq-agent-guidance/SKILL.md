---
name: tablinq-agent-guidance
description: guidance for agents working on the tablinq codebase. use when asked to refactor, add features, fix implementation quality issues, modify agent/browser automation behavior, or review code changes in tablinq. emphasizes small truthful changes, modular file structure, large-file avoidance, behavior preservation during refactors, and human-paced browser or gpt interactions.
---

# Tablinq Agent Guidance

Follow `AGENTS.md` as the baseline policy. Use this skill only for extra guidance on broad refactors, implementation quality, code review, file-size discipline, and browser or GPT interaction behavior.

## Truthfulness and evidence

- Never fake success, validation, coverage, or browser state.
- For documentation or agent-guidance work, describe only behavior supported by code, scripts, schemas, or tests.
- Remove stale instructions instead of preserving outdated operational stories.

## File-size discipline

- Do not keep expanding already-large files when a smaller cohesive module boundary is available.
- When editing a large file, look first for a safe split point: cohesive types, protocol mapping, command handling, selectors, persistence, UI interaction, tests, or pure helpers.
- Extract focused modules when that improves structure without changing requested behavior.
- Avoid broad formatting churn or cosmetic file moves.

## Implementation quality

- Prefer explicit data flow, typed contracts, and small cohesive modules.
- Use patterns only when they materially improve locality, testability, or change isolation.
- During broad refactors, preserve behavior unless the user explicitly asked to change it.

## Browser and GPT interaction

- Avoid robotic pacing; prefer readiness checks, bounded waits, retries, and small jitter only when human-like pacing matters.
- If interaction state is ambiguous, stop and gather evidence before proceeding.
- Prefer fixture or test coverage over live browser or GPT interaction unless live behavior is the point of the task.

## Code review focus

Prioritize regressions, hidden coupling, missing tests, oversized files, fake-seeming success paths, and browser interaction risk.
