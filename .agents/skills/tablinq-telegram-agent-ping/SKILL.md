---
name: tablinq-telegram-agent-ping
description: use at the end of every tablinq codex task to send a short telegram completion ping when files were modified, created, or deleted in the working directory, or when the answer took a long time because tests, builds, scripts, or other lengthy commands may have run. this skill is for local tablinq repo work and must stay token-efficient.
---

# Tablinq Telegram Agent Ping

At the end of every Tablinq task, decide quickly whether to ping Telegram.

Ping only when at least one condition is true:
- files were modified, created, deleted, moved, or renamed in the working directory
- the task took a long time because tests, builds, scripts, package commands, or other lengthy commands may have run

Do not ping for pure read-only answers unless the task was long-running.

If pinging, run this one-liner PowerShell 7 script from the repo root, passing one short text argument:

`pwsh -NoProfile -ExecutionPolicy Bypass -File .\.agents\skills\tablinq-telegram-agent-ping\scripts\send-agent-ping.ps1 "<Short summary`nContinuing summary on a new line>"`

Do not scan the script, simply run it (best-effort, fire and forget).

The message must be short: a few sentences only. Include:
- operation result
- most important facts
- touched file count
- inserted/removed line counts when available

Prefer git stats when available:
- touched files: `git diff --name-only`
- inserted/removed lines: `git diff --numstat`

Do not spend time reconstructing exact stats if unavailable. Use `unknown` or omit unavailable parts.

If `.agents\skills\tablinq-telegram-agent-ping\scripts\send-agent-ping.ps1` is missing, `.env.local` is missing, PowerShell is unavailable, Telegram is misconfigured, or the script exits non-zero, skip fast and do not retry.

Never print secrets. Never commit `.telegram-agent-ping`.