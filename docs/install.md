# Install

## Prerequisites

- Rust
- Node.js 20 or newer
- `pnpm`

Run `pnpm install` before packaging.

## Windows 10-11

1. Build a package with `scripts/package.ps1`.
2. Install it with `scripts/install.ps1`.
3. Load the unpacked extension from `%LOCALAPPDATA%\Tablinq\install\extension` in Edge or Chrome.
4. Start the daemon with `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe daemon start`.
5. Start the gateway with `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe gateway start`.
6. Open the managed browser with `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe browser open`.
7. Sign in once in the managed browser profile.
8. Confirm health with `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe status` or `%LOCALAPPDATA%\Tablinq\install\bin\gpt.exe doctor`.

Optional install flags:

- `scripts/install.ps1 -AutoStartDaemon`
- `scripts/install.ps1 -AutoStartGateway`
- `scripts/install.ps1 -AutoStartDaemon -AutoStartGateway`

What `scripts/install.ps1` does:

- installs the packaged layout under `%LOCALAPPDATA%\Tablinq\install`
- initializes the state root under `%LOCALAPPDATA%\Tablinq`
- registers the user-level native messaging host for Edge and Chrome
- writes startup helper scripts under `%LOCALAPPDATA%\Tablinq\install\startup`

## Unix-like systems

1. Build a package with `./scripts/package.sh`.
2. Install it with `./scripts/install.sh`.
3. Load the unpacked extension from `~/.local/share/tablinq/install/extension`.
4. Start the daemon with `~/.local/share/tablinq/install/bin/gpt daemon start`.
5. Start the gateway with `~/.local/share/tablinq/install/bin/gpt gateway start`.
6. Open the managed browser with `~/.local/share/tablinq/install/bin/gpt browser open`.
7. Sign in once in the managed browser profile.
8. Confirm health with `~/.local/share/tablinq/install/bin/gpt status` or `~/.local/share/tablinq/install/bin/gpt doctor`.

The Unix installer prepares install and state directories and writes startup helper scripts, but native-host registration remains Windows-first.

## Switching accounts

To sign into a different ChatGPT account in the managed profile, use one of:

- `gpt browser logout`
- `gpt browser switch-account`

Both commands clear the managed browser profile and reopen the managed ChatGPT tab.

## First-run checks

- `gpt status`
- `gpt doctor`
- `gpt capabilities`
- `gpt ask "hello from tablinq"`
