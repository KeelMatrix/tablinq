---
name: tablinq-agent-guidance
description: guidance for agents working on the tablinq codebase. use when asked to refactor, add features, fix implementation quality issues, modify agent/browser automation behavior, or review code changes in tablinq. emphasizes small truthful changes, modular file structure, large-file avoidance, behavior preservation during refactors, and human-paced browser or gpt interactions.
---

# Tablinq Agent Guidance

## Core Rules

Prefer correctness, truthfulness, and maintainability over speed. State uncertainty instead of guessing. Do not add speculative features, backward-compatibility layers, abstractions, or configuration unless the task requires them.

Keep changes local to the active execution path. Preserve existing behavior unless the user explicitly asks for a behavior change. Read nearby tests and contracts before editing, then validate with the narrowest relevant checks.

For documentation or agent-guidance work, only document behavior that is present in code, scripts, schemas, or tests. Remove stale instructions instead of preserving old operational stories.

## File Size Discipline

Do not create large files. Do not keep expanding already-large files when a smaller module boundary is available.

When editing a large file, first look for a safe split point: cohesive types, protocol mapping, command handling, selectors, persistence, UI interaction, tests, or pure helpers. Extract into focused modules without changing public behavior. Prefer several small, named modules over one broad utility file.

Avoid broad formatting churn. Move code only when the move improves structure or is needed for the requested change.

## Design Guidance

Use clear domain boundaries, explicit data flow, typed contracts, and small functions. Prefer simple design patterns that reduce duplication or isolate change. Do not introduce patterns by name only; use them only when they make the code easier to test and reason about.

Keep protocol, schema, and cross-layer changes synchronized only when required. Avoid cross-layer refactors unless they are necessary for the requested task.

## Browser and GPT Interaction

When automating browser or GPT interactions, avoid robotic pacing. Use bounded waits, jitter, retries, and state checks where appropriate. Prefer observable readiness over fixed sleeps, but add small randomized delays around clicks, typing, and submit actions when simulating human interaction matters.

Never fake success. If interaction state is ambiguous, report the ambiguity and collect evidence before proceeding.

## Output Expectations

Return minimal diffs and concise explanations. Mention validation performed or why it was not run. Do not include repo tours, speculative follow-ups, or unrelated cleanup.
