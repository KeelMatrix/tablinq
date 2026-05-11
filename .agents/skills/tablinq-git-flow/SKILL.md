---
name: tablinq-git-flow
description: Commit changes in the Tablinq repository with targeted validation, full-worktree staging, and commit messages that match the true staged diff.
---

Follow repository `AGENTS.md` first. This skill narrows commit behavior for this repo.

## When to use

Use this skill only when the user explicitly asks to commit, stage, push, open a PR, or otherwise perform git history work in the Tablinq repository. Do not create commits merely because files changed during a coding task.

## Commit rules

1. Before every commit, run `git status --short` and inventory every modified, deleted, renamed, staged, and untracked file.
2. Classify each path as:
   - current-task changes
   - pre-existing or user changes
   - generated/orphaned artifacts that should be cleaned or ignored
3. Do not create any commit unless the user explicitly requested a commit or git-history operation. If the user only asked for code changes, stop after validation and report the dirty worktree instead of committing.
4. Once you do create a commit, the worktree must end clean. Include compatible earlier changes in the same commit, or commit them separately when they deserve their own history entry.
5. If pre-existing changes are large, unrelated, or deserve their own history entry, commit them separately before or after the current-task commit without waiting for another instruction, because the original commit request already requires a clean final worktree. Use a commit message that truthfully describes those pre-existing changes.
6. Never silently skip pre-existing changes because the task mentions separate commits. "Commit each issue separately" means separate logical commits for each issue plus any needed separate pre-existing-change commit; it does not mean leave unrelated changes dirty.
7. Before every commit, run the repo's formatting checks and fix any reported issues.
8. Prefer the smallest validation loop that proves the change:
   - run targeted tests first
   - expand only when the change is broad, cross-cutting, or the narrow loop is not enough
9. For larger commits, multi-file changes, or substantial behavior changes, run local CI as well.
10. If the change set is large or touches several layers, use `pnpm run ci:local:full` and auto apply fixes until green.
11. Stage and commit the full intended worktree for that commit. Prefer full-worktree staging after the inventory, but split unrelated pre-existing changes into their own commit when needed.
12. After each commit, rerun `git status --short`. If anything remains dirty, either commit it in the correct separate commit, clean only clear generated/orphaned artifacts, or stop before making further commits and ask the user when the remaining files are strange, ambiguous, or likely should not be committed. These ask-before-commit cases should be rare.
13. Never create an empty commit.

## Commit message rules

- Write the message for the complete staged diff, not just the edits you made in this session.
- Assume other changes may have landed in the worktree while you were working.
- If unrelated or unexpected diffs are present, do not discard them and do not leave them dirty by default. Never rollback unexpected changes silently made by user; stop if you're not sure what to do.
- Include compatible unrelated diffs in the current commit, or create a separate commit for unrelated/pre-existing changes that deserve their own commit.
- Always tell the user which pre-existing changes were present and whether they were committed with the main change or in a separate commit.

## Branch and push rules

- Prefer current branch unless the user explicitly asks for branch or PR flow.
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
- whether pre-existing diffs were present
- whether pre-existing diffs were committed with the main change, committed separately, cleaned as artifacts, or intentionally left dirty by explicit user request
- how the commit message reflects the full staged scope
- confirmation that the worktree is clean if commits were made unless user allowed dirty diffs
