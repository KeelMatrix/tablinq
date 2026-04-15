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

You still sign in through your normal browser profile. Tablinq does not mint a direct API session behind your back.

## Windows 10-11

Build and install:

```powershell
scripts/package.ps1
scripts/install.ps1
```

Then:

1. Load the unpacked extension from `%LOCALAPPDATA%\Tablinq\install\extension` into your normal Edge or Chrome profile.
2. Start the daemon:
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe daemon start`
3. Start the gateway:
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe gateway start`
4. Fully close Edge or Chrome so the debug flags can take effect.
5. Start your normal browser in attach mode:
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe browser start-debug`
6. Sign in manually at `https://chatgpt.com/` in that browser profile.
7. Attach and confirm health:
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe browser attach`
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe browser connection-info`
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

1. Load the unpacked extension from `~/.local/share/tablinq/install/extension` into your normal Chrome or Edge profile.
2. Start the daemon:
   - `~/.local/share/tablinq/install/bin/gpt daemon start`
3. Start the gateway:
   - `~/.local/share/tablinq/install/bin/gpt gateway start`
4. Fully close Chrome or Edge so the debug flags can take effect.
5. Start your normal browser in attach mode:
   - `~/.local/share/tablinq/install/bin/gpt browser start-debug`
6. Sign in manually at `https://chatgpt.com/` in that browser profile.
7. Attach and confirm health:
   - `~/.local/share/tablinq/install/bin/gpt browser attach`
   - `~/.local/share/tablinq/install/bin/gpt browser connection-info`
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
- a detected attachable browser
- extension connectivity
- an authenticated normal-browser session

## Switching accounts

If the normal browser is signed into the wrong ChatGPT account, sign out in that browser tab, sign back in manually, then run:

- `gpt browser attach`
- `gpt browser open`

If your machine or profile cannot expose a local debug endpoint, switch to explicit managed fallback by setting `browser_connection_mode = "managed_only"` in `config/config.toml`, then use `gpt browser open`.

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
