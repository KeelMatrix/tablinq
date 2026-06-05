---
name: tablinq-windows-command-workarounds
description: Use when Tablinq work on Windows hits or may hit command-environment failures involving git staging or commits, .git/index.lock permission denied, RTK or pnpm EPERM, Cargo target write errors, Vitest browser-test flakes, scripts/ci-local.ps1, PowerShell version mismatches, or Telegram ping execution.
---

# Tablinq Windows Command Workarounds

Use this skill only for Windows command execution problems in this repo. Keep the loop short: identify the known failure, apply the known workaround, then continue.

## Known fixes

- Cargo cannot write under repo `target/`: set `$env:CARGO_TARGET_DIR = "C:\tmp\tablinq-target"`.
- `rtk pnpm ...` or `pnpm ...` fails with `EPERM` around `C:\Users\rdime`: call `C:\nvm4w\nodejs\pnpm.ps1` directly.
- Vitest full browser suite flakes under default parallelism: use `pnpm exec vitest run --maxWorkers=4`.
- Targeted Vitest file hangs or runs too broadly: use `pnpm exec vitest run <file> --reporter=dot`.
- Telegram ping fails under Windows PowerShell 5.1: run it with `pwsh`.
- Git cannot create `.git/index.lock`: first check `Test-Path .git\index.lock`. If false, check ACLs with `icacls .git | Select-Object -First 8`; do not claim stale lock.

## Git ACL deny

If `.git/index.lock` does not exist and `git add` fails with permission denied, look for explicit deny ACEs:

```powershell
icacls .git | Select-Object -First 8
```

If there is a raw SID deny entry, Git is blocked by Windows ACL/sandbox state, not repo corruption. Do not keep retrying `git add`.

## CI

For full local CI on Windows:

```powershell
$env:CARGO_TARGET_DIR = "C:\tmp\tablinq-target"
rtk powershell -ExecutionPolicy Bypass -File scripts/ci-local.ps1 -Job all
```

If full CI fails in a browser test but the failing file passes in isolation, treat it as likely parallelism/load-related before changing product code.
