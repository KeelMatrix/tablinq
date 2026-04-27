---
name: tablinq-test-selection
description: Choose the smallest valid validation loop in the Tablinq repository. Use when Codex needs to decide which test, typecheck, build, or live command to run after a change without wasting time on broad suites.
---

Follow the repository `AGENTS.md` as the baseline policy. This skill only adds validation-selection guidance.

## Selection flow

1. Identify the owning crate or package for the change.
2. Prefer the narrowest command that exercises that owner.
3. Escalate only if:
   - the narrow test does not exist
   - the result is inconclusive
   - the change crosses a real contract boundary

## Preferred validation choices

- CLI-local Rust changes: `cargo test -p gpt`
- Daemon-local Rust changes: `cargo test -p gptd`
- Gateway-local Rust changes: `cargo test -p tablinq-gateway`
- Native host Rust changes: `cargo test -p tablinq-host`
- Browser adapter TS changes: matching `pnpm test packages/browser-adapter/test/<file>.test.ts`
- Browser adapter shard checks: `pnpm run test:browser-adapter:foundation`, `pnpm run test:browser-adapter:ask`, `pnpm run test:browser-adapter:research`, or `pnpm run test:browser-adapter:recovery`
- Extension TS changes: matching `pnpm test packages/extension/test/<file>.test.ts`
- Schema/probe/content-script TS checks: `pnpm run test:ts:fast`
- TS surface-wide type validation: `pnpm lint`
- TS package build validation when necessary: `pnpm build`
- Windows CI parity when needed: `pnpm run ci:local`, `pnpm run ci:local:all`, or `pnpm run ci:local:package-smoke`

## Escalation rules

- Do not jump to `cargo test` or `pnpm test` for a local change unless the narrower loop cannot prove correctness.
- If no automated test covers the path, use the smallest live command that does and state that gap explicitly.

## Output

State:

- why the chosen command is the smallest valid loop
- whether broader validation is still outstanding
