# Scripts Guide

This folder contains the repo's operational helper scripts.

The scripts are intentionally split by responsibility:

- **bootstrap** checks whether a machine is ready to work on the repo.
- **ci-local** mirrors the current GitHub Actions jobs as closely as a local machine can.
- **test** validates the repo.
- **package** builds a distributable layout.
- **install** installs that packaged layout onto a machine.
- **soak_gateway** stress-checks the local OpenAI-compatible gateway.

That separation matters. Packaging and installation are not the same step, and validation is kept separate from both so contributors can run the cheapest command that answers their immediate question.

## Which script should I use?

| Goal                                                        | Use                           |
| ----------------------------------------------------------- | ----------------------------- |
| Confirm your Windows shell can see Node, pnpm, and cargo    | `bootstrap.ps1`               |
| Mirror the current GitHub Actions jobs locally on Windows   | `ci-local.ps1`                |
| Run the broad local test suite                              | `test.ps1` or `test.sh`       |
| Run the fast Python developer-test orchestrator directly    | `test_local.py`               |
| Build a distributable package without installing it         | `package.ps1` or `package.sh` |
| Install Tablinq locally from the repo/package output        | `install.ps1` or `install.sh` |
| Repeatedly hit the local gateway to check stability/latency | `soak_gateway.ps1`            |

## Principles behind these scripts

- **Fail fast.** Most scripts enable strict error handling immediately.
- **One main job per script.** Each file exists for a distinct stage in the developer or operator workflow.
- **Cross-platform where practical.** Packaging, install, and validation have both PowerShell and shell variants where the repo supports them.
- **Human-readable output.** Scripts print the next useful action rather than hiding state.
- **Operational safety.** Install scripts validate paths, required commands, and expected outputs before treating a run as successful.

## Script reference

### `bootstrap.ps1`

**What it does**

- Adds the local Cargo bin directory to the current PowerShell session `PATH` when present.
- Prints the detected `node`, `pnpm`, and `cargo` versions.
- Points the user at the main next-step scripts for install and packaging.

**Why it exists**

This is the cheapest Windows preflight check in the repo. It lets a contributor verify the toolchain before they spend time on dependency install, packaging, or debugging a missing command.

**Use it when**

- A Windows machine is being set up for the first time.
- PowerShell cannot find Rust tooling.
- You want a quick environment sanity check before running heavier scripts.

**Do not use it for**

- Building, testing, or installing the product.

---

### `test.ps1`

**What it does**

Runs the broad local validation orchestrator:

1. Rust package tests for `gpt`, `gptd`, `shared-protocol`, and `tablinq-host` with `--locked`
2. Gateway tests in focused module chunks, so browser-backed tests report progress instead of disappearing inside one long workspace run
3. `pnpm exec vitest run --reporter=dot`

It also prepends Cargo's bin directory to the current PowerShell session `PATH` when available.
If `node_modules` is missing, it first runs `pnpm install --frozen-lockfile`.
The implementation delegates to `test_local.py`, which runs independent jobs in parallel and keeps browser-heavy gateway chunks serialized.
The default profile is balanced: Rust core and fast gateway tests run in parallel, TypeScript runs in its own phase, and browser-heavy gateway chunks run one at a time. `pnpm run test:local:aggressive` is available for experimentation, but it can expose local browser/resource contention.

**Why it exists**

It gives Windows contributors a single broad local test command with useful progress and logs. It includes the browser-heavy gateway chunks, but static checks, formatting, build validation, and package smoke coverage belong to `ci-local.ps1`.

**Use it when**

- You want broad Rust and TypeScript test coverage without manually coordinating gateway chunks.
- You are preparing a commit or pull request from Windows.
- You changed code across Rust and TypeScript boundaries.

**Do not use it for**

- Producing a distributable package.
- Installing the app onto a machine.
- Package smoke validation.

---

### `test_local.py`

**What it does**

Runs the cross-platform developer validation orchestrator used by `test.ps1`, `test.sh`, and the root `test:local*` package scripts. It creates per-job logs in a temporary directory, runs independent Rust and gateway jobs with bounded parallelism, keeps browser-heavy gateway chunks serialized, and supports `--profile balanced`, `--profile aggressive`, and `--keep-logs`.

**Why it exists**

The Rust and TypeScript test surfaces have different costs and contention profiles. Keeping the orchestration in one Python entrypoint avoids duplicating scheduling logic across PowerShell and shell wrappers.

**Use it when**

- You need the same local validation behavior from a non-wrapper environment.
- You want `--keep-logs` for diagnosing a failing local test job.
- You are adjusting local test scheduling itself.

**Do not use it for**

- CI job parity. Use `ci-local.ps1` on Windows for that.
- Packaging, installation, or gateway soak checks.

---

### `ci-local.ps1`

**What it does**

Runs the current GitHub job commands locally on Windows, with the same environment variables and command ordering used in workflow files:

- `rust`
  - `cargo fmt --all --check`
  - `cargo clippy --workspace --all-targets --locked -- -D warnings`
  - `cargo test --workspace --locked` with browser-integration and package-smoke tests skipped through environment flags
- `typescript`
  - `pnpm install --frozen-lockfile`
  - `pnpm format`
  - `pnpm lint`
  - `pnpm build`
  - `pnpm run test:ts:fast`
- `browser-adapter`
  - `pnpm run test:browser-adapter:foundation`
  - `pnpm run test:browser-adapter:ask`
  - `pnpm run test:browser-adapter:research`
  - `pnpm run test:browser-adapter:recovery`
- `package-smoke`
  - `pnpm install --frozen-lockfile`
  - `./scripts/package.ps1 -Profile dev`
  - `cargo test -p gpt --test production_smoke windows_packaging_and_install_smoke_cover_packaged_layout -- --nocapture`

Composite jobs:

- `quick`: `rust-fmt` plus `typescript`
- `all` / `full`: `rust`, `rust-browser-recovery`, `typescript`, `typescript-test`, and `package-smoke`
- `browser-adapter`: all browser-adapter shards

Focused jobs:

- `rust-fmt`
- `rust-clippy`
- `rust-test`
- `typescript-format`
- `typescript-test-shard-1` through `typescript-test-shard-10`
- `browser-adapter-foundation`
- `browser-adapter-ask`
- `browser-adapter-research`
- `browser-adapter-recovery`
- `package-smoke`

Use `-Job full` or `pnpm run ci:local:full` when you deliberately want full local parity with both GitHub workflow files. Use `typescript-test-shard-N` when reproducing a specific `typescript-tests` CI shard locally.

**Why it exists**

The repo already has general validation helpers, but GitHub now has three distinct jobs with slightly different command surfaces. This script gives Windows developers a one-command way to mirror those jobs locally without mentally translating workflow YAML into shell commands.

**Use it when**

- You want the closest local equivalent to the current GitHub Actions jobs.
- You are debugging a failure that happened in GitHub CI and want to replay the exact job commands.
- You want to run only one CI job locally instead of the broader local validation script.

**Do not use it for**

- Cross-platform Unix validation.
- Packaging or installation outside the CI-mirroring workflow.

---

### `test.sh`

**What it does**

Runs the Unix equivalent of the broad local test stack:

1. Rust package tests for `gpt`, `gptd`, `shared-protocol`, and `tablinq-host` with `--locked`
2. Gateway tests in focused module chunks, so browser-backed tests report progress instead of disappearing inside one long workspace run
3. `pnpm exec vitest run --reporter=dot`

If `node_modules` is missing, it first runs `pnpm install --frozen-lockfile`.
The implementation delegates to `test_local.py`, which runs independent jobs in parallel and keeps browser-heavy gateway chunks serialized.
The default profile is balanced: Rust core and fast gateway tests run in parallel, TypeScript runs in its own phase, and browser-heavy gateway chunks run one at a time. Use `python3 scripts/test_local.py --profile aggressive` only when you want to probe local parallelism limits.

**Why it exists**

This is the Unix/Linux counterpart to `test.ps1`, giving non-Windows environments the same one-command broad test path without duplicating CI-only static checks or package smoke validation.

**Use it when**

- You are validating the repo on Linux or another Unix-like environment.
- You want broad Rust and TypeScript test coverage.

**Do not use it for**

- Packaging or installation.
- Package smoke validation.

---

### `package.ps1`

**What it does**

Builds the Windows package layout under `dist/package/tablinq` by:

- compiling the Rust binaries (`gpt`, `gptd`, `tablinq-gateway`, `tablinq-host`),
- building the browser adapter and extension,
- deploying browser-adapter assets,
- copying binaries, extension assets, selector packs, schemas, docs, license, and root README into the package layout,
- deriving a stable extension ID from the extension manifest key,
- writing `package-metadata.json` from `gpt version --json` output.

**Why it exists**

Installation should consume a predictable packaged layout rather than raw repo outputs. This script creates that installable boundary and captures package metadata needed by downstream install and operational flows.

**Use it when**

- You need `dist/package/tablinq` for local install or artifact creation.
- You want to smoke-test the packaged Windows layout.
- You need a reproducible package directory that mirrors what installation expects.

**Do not use it for**

- Running tests.
- Installing onto a target machine directly.

**Notes**

- Supports `-Profile` and optional `-TargetDir`.
- This is the script wired into the repo's `package:windows` npm script.

---

### `package.sh`

**What it does**

Builds the Unix package layout under `dist/package/tablinq` by:

- compiling the same Rust binaries,
- building browser-adapter and extension assets,
- deploying the browser-adapter,
- copying runtime assets and docs into the package directory,
- deriving the extension ID from the manifest key,
- generating `package-metadata.json` from `gpt version --json` output.

**Why it exists**

It provides the same package-stage boundary as `package.ps1`, but for Unix environments. Keeping packaging explicit avoids coupling install logic to build logic more than necessary.

**Use it when**

- You need a package layout on Linux/Unix.
- You want to inspect or distribute the packaged directory before installation.

**Do not use it for**

- Environment bootstrap.
- Full repo validation.

**Notes**

- Accepts the build profile as the first positional argument.
- Supports `OUTPUT_ROOT` and `CARGO_TARGET_DIR` overrides through the environment.

---

### `install.ps1`

**What it does**

This is the Windows all-in-one local install path. It can:

- verify required commands and writable locations,
- optionally install dependencies,
- optionally invoke `package.ps1`,
- stage and replace the installed layout under `%LOCALAPPDATA%\Tablinq\install`,
- validate that expected binaries and extension files exist,
- update config and clear scheduled-task and Run-key autostart persistence,
- register the native host unless skipped,
- enable logon-time scheduled-task autostart when `-EnableAutoStart` is passed,
- optionally update PATH,
- launch the daemon and gateway by default unless `-NoLaunchAfterInstall` is passed,
- print a short status summary plus the remaining manual browser/extension steps.

It also contains defensive handling for blocked files, registry access, and antivirus-related failure modes.

**Why it exists**

A packaged layout is still not a usable local install. Windows needs extra machine-specific work such as placement, registry/native-host registration, startup setup, launch behavior, and operator guidance. This script centralizes that operational complexity in one place.

**Use it when**

- You want the main Windows install flow from repo checkout to working local install.
- You are smoke-testing installation behavior, not just packaging.
- You need the script that matches the root README's Windows-first install path.

**Do not use it for**

- A lightweight preflight check.
- Pure packaging when no install is needed.
- Gateway stress testing.

**Notes**

- Supports several `-Skip...` switches so operators can bypass dependency install, package build, registry changes, autostart setup, or PATH updates.
- Uses `-NoLaunchAfterInstall` only when you explicitly do not want the script to start services immediately.
- Prefer this over calling lower-level steps manually unless you are debugging a specific install stage.

---

### `install.sh`

**What it does**

This is the Unix/Linux install path. It:

- validates required commands and writable directories,
- resolves a supported browser path/channel,
- stages and replaces the installed layout,
- ensures config exists,
- writes config values such as browser path and channel,
- writes startup helper scripts,
- attempts to start the daemon and gateway via the installed `gpt` binary,
- prints follow-up manual steps for attach-first browser setup.

**Why it exists**

Unix installation has a different operational model than Windows. It still needs more than packaging: install roots, state roots, config creation, browser discovery, process cleanup, and startup helper generation all belong in an installer, not in the package step.

**Use it when**

- You want a local Unix/Linux install from the packaged layout.
- You need a repeatable install flow that prepares config and startup scripts.

**Do not use it for**

- Validation-only work.
- Windows-native installation concerns such as registry setup.

**Notes**

- Uses `PACKAGE_ROOT`, `INSTALL_ROOT`, and `STATE_ROOT` environment overrides.
- Detects Chrome/Chromium/Edge-style browser executables and writes those settings into config.

---

### `soak_gateway.ps1`

**What it does**

Sends repeated POST requests to the local OpenAI-compatible `chat/completions` endpoint with incrementing request IDs, measures elapsed time, and prints success/failure information for each request.

Configurable inputs include:

- `BaseUrl`
- `ApiKey`
- `Model`
- `Count`
- `Prompt`

**Why it exists**

This is not a developer build or install helper. It is an operational confidence check for the local gateway. It helps answer whether the gateway remains stable over repeated sequential requests and whether latency or reliability regressions are obvious.

**Use it when**

- You want a quick soak/stability check against a running local gateway.
- You are debugging request handling, auth headers, request IDs, or response timing.

**Do not use it for**

- Functional repo validation.
- Packaging or installation.

---

## Recommended workflow

### Windows contributor

1. Run `bootstrap.ps1`.
2. Run `test.ps1` while iterating on broad code changes.
3. Run `ci-local.ps1 -Job quick` for the default fast CI mirror.
4. Run `ci-local.ps1 -Job all` or `ci-local.ps1 -Job full` when you need full CI parity including package smoke.
5. Run `ci-local.ps1 -Job package-smoke` when packaging or installed behavior changed and you do not need the rest of CI.
6. Run `package.ps1` when you need an installable layout.
7. Run `install.ps1` when you need a real local install.
8. Run `soak_gateway.ps1` only after the local gateway is up and you want runtime confidence.

### Unix/Linux contributor

1. Run `test.sh` while iterating on code.
2. Run `package.sh` when you need the packaged layout.
3. Run `install.sh` when you need a machine-local install.

## What not to add here later

To keep this folder understandable, future scripts should follow the same rules:

- Prefer a **single clear job** over multi-purpose "do everything" helpers.
- Add platform-specific variants only when the operational steps are genuinely different.
- Keep build/package/install/test/ops concerns separated.
- Document new scripts here as soon as they are added.
