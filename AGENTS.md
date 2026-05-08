## Navigation

- Bug: start at failing command surface: `crates/gpt/src/*` -> `crates/gptd/src/*` -> `packages/browser-adapter/src/*` -> `packages/extension/src/*`
- Feature: start at user-facing entry: CLI in `crates/gpt`, daemon flow in `crates/gptd`, gateway in `crates/tablinq-gateway`, native host in `crates/tablinq-host`, browser/runtime in `packages/browser-adapter`, page bridge in `packages/extension`
- CLI/task routing and terminal presentation: `crates/gpt/src/main.rs`, `crates/gpt/src/lib.rs`, `crates/gpt/src/terminal/*`
- Daemon/state/browser orchestration: `crates/gptd/src/*`
- Gateway HTTP surface: `crates/tablinq-gateway/src/*`
- Native messaging bridge: `crates/tablinq-host/src/*`
- Stable types/contracts: `crates/shared-protocol`, `schemas/v1`
- Browser runtime: `packages/browser-adapter/src/*`
- Extension probes/bridge: `packages/extension/src/*`
- Test harness for browser flows: `packages/fake-chatgpt`
- Selector data: `selector-packs/v1`
- Preferred search order: exact command/file -> owning crate/package entry -> matching tests -> shared protocol/schema -> selector pack/runtime

## Ignore

- Ignore: `target/`
- Ignore: `node_modules/`
- Ignore: `.pnpm-store/`
- Ignore: `dist/`
- Ignore: `build/`
- Ignore: `coverage/`
- Ignore: `playwright-report/`
- Ignore: `test-results/`
- Ignore: `logs/`
- Ignore: `.vscode/`
- Ignore: `.vs/`
- Ignore: `.idea/`
- Ignore: `data/`
- Ignore: `downloads/`
- Ignore: `browser-profile/`
- Ignore: `.tmp-cli-smoke/`
- Ignore: `tmp/`
- Ignore: `temp/`
- Ignore: `AppData/Local/Temp/`
- Ignore: `packages/browser-adapter/dist/`
- Ignore: `packages/extension/dist/`
- Ignore: `scripts/` unless task is packaging/install/tooling
- Ignore: `packages/fake-chatgpt/` unless task is test/harness/UI-fixture work
- Ignore: `selector-packs/` unless task is selector work
- Ignore: `schemas/` unless task is protocol/schema work
- Ignore: `.agents/skills/tablinq-telegram-agent-ping/scripts/.env.local`

## Commands

- Install: `pnpm install`
- Rust tests: `cargo test`
- TS tests: `pnpm test`
- Fast TS core validation: `pnpm run test:ts:fast`
- Browser-adapter shards: `pnpm run test:browser-adapter:foundation`, `pnpm run test:browser-adapter:ask`, `pnpm run test:browser-adapter:research`, `pnpm run test:browser-adapter:recovery`
- TS typecheck: `pnpm lint`
- TS build: `pnpm build`
- Format check: `cargo fmt --all --check` and `pnpm format`
- Local CI mirror: `pnpm run ci:local`
- Full CI parity mirror: `pnpm run ci:local:all` or `pnpm run ci:local:full`
- Package smoke mirror: `pnpm run ci:local:package-smoke`
- Start daemon: `cargo run -p gpt -- daemon start`
- Check daemon: `cargo run -p gpt -- daemon status`
- Start gateway: `cargo run -p gpt -- gateway start`
- Check gateway: `cargo run -p gpt -- gateway status`
- Doctor: `cargo run -p gpt -- doctor`
- Browser attach-first: `cargo run -p gpt -- browser start-debug` -> `cargo run -p gpt -- browser attach`
- Fastest Rust validation: `cargo test -p gpt`
- Fastest daemon validation: `cargo test -p gptd`
- Fastest gateway validation: `cargo test -p tablinq-gateway`
- Fastest native host validation: `cargo test -p tablinq-host`
- Fastest browser-adapter validation: `pnpm test packages/browser-adapter/test/<file>.test.ts`
- Fastest extension validation: `pnpm test packages/extension/test/<file>.test.ts`
- Fastest schema/probe/content-script validation: `pnpm run test:ts:fast`
- Package only when required: `scripts/package.ps1` or `./scripts/package.sh`

## Constraints

- Modify only files on the active execution path
- Prefer one crate/package at a time
- Keep protocol/schema changes synchronized only when required
- Do not touch selector packs for non-selector work
- Do not touch packaging/install scripts unless directly implicated
- Avoid cross-layer refactors unless requested
- Preserve attach-first browser model unless task requires change
- Preserve command surface and package layout unless task requires change
- Do not add backward-compatibility layers unless explicitly asked
- Keep edits minimal, local, and test-backed

## Efficiency Rules

- Do not scan the whole repo
- Read root manifests first: `Cargo.toml`, `package.json`, `README.md`
- Use `docs/architecture.md` before broad runtime search
- Jump directly to owning entry points
- Use file-path search, symbol search, and test names before broad text search
- Read matching tests before expanding search radius
- Stop after identifying the owning layer
- Do not reopen files already ruled out
- Do not rerun unchanged commands
- Run narrow tests before suite-wide tests
- Escalate from file test -> package test -> full suite only if needed

## Output

- Return minimal diffs only
- Minimal explanations
- No repo tours
- No speculative changes
- No broad formatting churn
- Briefly mention what global / local SKILLS were used and why

## Agent skills

### Issue tracker

Issues and PRDs are tracked in GitHub Issues for `KeelMatrix/tablinq`. See `docs/agents/issue-tracker.md`.

### Triage labels

The default five-label triage vocabulary is used. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context domain docs are expected at the repo root. See `docs/agents/domain.md`.

### Skill selection

For Tablinq tasks where the user did not explicitly name a skill, start with `tablinq-skill-selector` to choose the narrowest relevant repo-local and installed engineering skills.

### Telegram agent ping

Always use the repo-local `tablinq-telegram-agent-ping` skill at the end of every task.
