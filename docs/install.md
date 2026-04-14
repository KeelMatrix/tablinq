# Install

This guide covers the packaged install path for Tablinq. If you want the contributor workflow instead, see [development.md](development.md).

## Prerequisites

- Rust
- Node.js 20 or newer
- `pnpm`

Install workspace dependencies first:

```bash
pnpm install
```

## What you are installing

A normal Tablinq install gives you:

- the `gpt`, `gptd`, `tablinq-gateway`, and `tablinq-host` binaries
- the browser adapter runtime
- the unpacked browser extension
- a local state root for config, logs, SQLite state, downloads, uploads, and crash bundles

You still sign in through a real managed browser profile. Tablinq does not mint a direct API session behind your back.

## Windows 10-11

Build and install:

```powershell
scripts/package.ps1
scripts/install.ps1
```

Then:

1. Load the unpacked extension from `%LOCALAPPDATA%\Tablinq\install\extension` in Edge or Chrome.
2. Start the daemon:
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe daemon start`
3. Start the gateway:
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe gateway start`
4. Open the managed browser:
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe browser open`
5. Sign in once in the managed browser profile.
6. Confirm health:
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe status`
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe doctor`

Optional installer flags:

- `scripts/install.ps1 -AutoStartDaemon`
- `scripts/install.ps1 -AutoStartGateway`
- `scripts/install.ps1 -AutoStartDaemon -AutoStartGateway`

What the Windows installer does:

- installs the packaged layout under `%LOCALAPPDATA%\Tablinq\install`
- initializes the state root under `%LOCALAPPDATA%\Tablinq`
- registers the user-level native messaging host for Edge and Chrome
- writes startup helper scripts under `%LOCALAPPDATA%\Tablinq\install\startup`

## Unix-like systems

Build and install:

```bash
./scripts/package.sh
./scripts/install.sh
```

Then:

1. Load the unpacked extension from `~/.local/share/tablinq/install/extension`.
2. Start the daemon:
   - `~/.local/share/tablinq/install/bin/gpt daemon start`
3. Start the gateway:
   - `~/.local/share/tablinq/install/bin/gpt gateway start`
4. Open the managed browser:
   - `~/.local/share/tablinq/install/bin/gpt browser open`
5. Sign in once in the managed browser profile.
6. Confirm health:
   - `~/.local/share/tablinq/install/bin/gpt status`
   - `~/.local/share/tablinq/install/bin/gpt doctor`

The Unix installer prepares install and state directories and writes startup helper scripts. Native-host registration remains Windows-first.

## First-run checks

After install, run:

- `gpt status`
- `gpt doctor`
- `gpt capabilities`
- `gpt ask "hello from tablinq"`

You want to see:

- a running daemon
- a running gateway
- a detected browser
- extension connectivity
- an authenticated managed session

## Switching accounts

If the managed browser is signed into the wrong ChatGPT account, use one of:

- `gpt browser logout`
- `gpt browser switch-account`

Both commands clear the managed profile and reopen the managed tab so you can sign in cleanly again.

## Where state lives

The state root contains:

- `config/config.toml`
- `logs/`
- `db/`
- `browser-profile/`
- `downloads/`
- `crash-bundles/`
- `gateway-uploads/`

If something goes wrong, check [troubleshooting.md](troubleshooting.md).
