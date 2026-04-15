# Install

This guide covers the packaged install path for Tablinq. For repo-checkout contributor work, see [development.md](development.md).

## The model you are installing

Tablinq is attach-first.

The intended production workflow is:

- install Tablinq
- load the Tablinq extension into your normal Chrome or Edge profile
- start the daemon and gateway
- start your normal browser in local-debug mode with `gpt browser start-debug`
- sign in manually at `chatgpt.com`
- let Tablinq attach to that existing browser session with `gpt browser attach`

The old Playwright-owned managed-login flow is not the normal path anymore. It was abandoned as the default because Cloudflare and anti-bot protection at `chatgpt.com` made automation-owned login and startup the wrong operational model. Tablinq still uses Playwright internally for attach, screenshots, uploads, downloads, and recovery, but your actual ChatGPT session is expected to be a user-owned browser session.

## Prerequisites

- Rust
- Node.js 20 or newer
- `pnpm`

Install workspace dependencies first:

```bash
pnpm install
```

## What gets installed

A normal package install gives you:

- `gpt`, `gptd`, `tablinq-gateway`, and `tablinq-host`
- the browser adapter runtime
- the unpacked browser extension
- the selector-pack data
- local config, logs, SQLite state, downloads, uploads, and crash-bundle storage

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
4. Fully close Edge or Chrome so the debug flags can take effect cleanly.
5. Start your normal browser in attachable mode:
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe browser start-debug`
6. Sign in manually at `https://chatgpt.com/` in that same browser profile.
7. Attach and verify:
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe browser attach`
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe browser connection-info`
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe status`
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe doctor`
   - `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe capabilities`

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
4. Fully close Chrome or Edge so the debug flags can take effect cleanly.
5. Start your normal browser in attachable mode:
   - `~/.local/share/tablinq/install/bin/gpt browser start-debug`
6. Sign in manually at `https://chatgpt.com/` in that same browser profile.
7. Attach and verify:
   - `~/.local/share/tablinq/install/bin/gpt browser attach`
   - `~/.local/share/tablinq/install/bin/gpt browser connection-info`
   - `~/.local/share/tablinq/install/bin/gpt status`
   - `~/.local/share/tablinq/install/bin/gpt doctor`
   - `~/.local/share/tablinq/install/bin/gpt capabilities`

The Unix installer prepares install and state directories and writes startup helper scripts. Native-host registration remains Windows-first.

## First-run checks

After install, run:

- `gpt status`
- `gpt doctor`
- `gpt browser connection-info`
- `gpt capabilities`
- `gpt ask "hello from tablinq"`

You want to see:

- a running daemon
- a running gateway
- a reachable local debug endpoint
- a connected extension
- an authenticated signed-in ChatGPT tab

## Fallback mode

Use managed fallback only when attach mode cannot be enabled on this machine or browser profile, or when you need debugging, CI, or harness isolation.

Set this in `config/config.toml`:

```toml
browser_connection_mode = "managed_only"
```

Then use:

- `gpt browser open`

`gpt browser reset-profile` is only for managed fallback recovery. It is not part of the recommended attach-first flow.

## Antivirus warning on Windows

Local packaging, testing, installation, and browser startup can be blocked by antivirus products. If you see `Access is denied`, blocked-process notifications, or quarantined binaries, add the relevant paths to your antivirus exceptions and restore any already-blocked files from notifications or quarantine.

Example Windows paths that may require exceptions in a typical local environment:

- `C:\Users\user\Desktop\gpt-cli`
- `C:\Users\user\AppData\Local\Tablinq`
- `C:\Users\user\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup`
- `C:\nvm4w\nodejs`
- `C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe`
- `C:\nvm4w\nodejs\node.exe`
- `C:\Users\user\Desktop\gpt-cli\scripts`
- `C:\Users\user\AppData\Local\nvm`
- `C:\Program Files\PowerShell\7\pwsh.exe`
- `C:\Program Files\Google\Chrome\Application\chrome.exe`
- `C:\Users\user\AppData\Local\nvm\v20.19.3\node.exe`
- `C:\Windows\System32\cmd.exe`
- `C:\Users\user\Desktop\gpt-cli\node_modules\.pnpm\@esbuild+win32-x64@0.27.7\node_modules\@esbuild\win32-x64\esbuild.exe`
- `C:\Users\user\Desktop\gpt-cli\node_modules`

Adjust those paths for your own install directory, browser, and Node version.

## State layout

The state root contains:

- `config/config.toml`
- `logs/`
- `db/`
- `browser-profile/`
- `downloads/`
- `crash-bundles/`
- `gateway-uploads/`

`browser-profile/` still exists even though attach-first no longer uses a managed profile as the default ownership model. That directory remains useful for managed fallback and compatibility.

If something goes wrong, see [troubleshooting.md](troubleshooting.md).
