---
name: tablinq-git-flow
description: Commit changes in the Tablinq repository with targeted validation, full-worktree staging, and commit messages that match the true staged diff.
---

Follow repository `AGENTS.md` first. This skill narrows commit behavior for this repo.

## When to use

Use this skill when preparing a commit in the Tablinq repository.

## Commit rules

1. Before every commit, run the repo's formatting checks and fix any reported issues.
2. Prefer the smallest validation loop that proves the change:
   - run targeted tests first
   - expand only when the change is broad, cross-cutting, or the narrow loop is not enough
3. For larger commits, multi-file changes, or substantial behavior changes, run local CI as well.
4. If the change set is large or touches several layers, use `pnpm run ci:local:full`.
5. Stage and commit the full working tree, not just the latest edits, unless the user explicitly says otherwise.
6. Never create an empty commit.

## Commit message rules

- Write the message for the complete staged diff, not just the edits you made in this session.
- Assume other changes may have landed in the worktree while you were working.
- If unrelated or unexpected diffs are present, do not discard them.
- Include those diffs in the commit, adjust the message to match the true scope, and tell the user what else was included.

## Branch and push rules

- Prefer `main` unless the user explicitly asks for branch or PR flow.
- Before pushing, rerun the smallest relevant validation if new commits landed or the worktree changed.
- If packaging, gateway, or browser-adapter behavior changed, prefer `pnpm run ci:local:full` before push.
- If unrelated staged diffs are included, mention them explicitly in the push or PR summary.

## Validation order

- Start with targeted tests for the owning crate or package.
- Run broader package tests only when needed.
- Run local CI for broad changes, risky changes, or when preparing a commit that spans multiple files or layers.
- Prefer repo scripts when they exist.

## Minimum expected commands

- Formatting: `cargo fmt --all --check` and `pnpm format`
- Targeted validation: choose the smallest relevant Rust or TS test command
- Broad validation when warranted: `pnpm run ci:local:full`

## Output expectations

When using this skill, report:

- what validation was run
- whether the commit includes unrelated diffs
- how the commit message reflects the full staged scope
