# Install

This guide covers the packaged install path for Tablinq. For repo-checkout contributor work, see [development.md](development.md).

## The model you are installing

Tablinq is attach-first.

The intended production workflow is:

- install Tablinq
- let the installer bring up the daemon and gateway
- start an attachable browser session with `gpt browser start-debug`
- load the Tablinq extension into the target Chrome or Edge profile
- sign in manually at `chatgpt.com`
- let Tablinq attach to that existing browser session with `gpt browser attach`

The old Playwright-owned managed-login flow is not the normal path anymore. It was abandoned as the default because Cloudflare and anti-bot protection at `chatgpt.com` made automation-owned login and startup the wrong operational model. Tablinq still uses Playwright internally for attach, screenshots, uploads, downloads, and recovery, but your actual ChatGPT session is expected to be a user-owned browser session.

On current Chrome, `gpt browser start-debug` launches a dedicated persistent real Chrome profile under the Tablinq state root so Chrome will expose the local debug endpoint. This remains the attach-first model, not managed fallback. You need to sign in once in that dedicated Chrome attach profile before running `gpt browser attach`.

## Prerequisites

- Rust
- Node.js 20 or newer
- `pnpm`

## What gets installed

A normal package install gives you:

- `gpt`, `gptd`, `tablinq-gateway`, and `tablinq-host`
- the browser adapter runtime
- the unpacked browser extension
- the selector-pack data
- local config, logs, SQLite state, downloads, uploads, and crash-bundle storage

## Windows 10-11

From a repo checkout, run the one entry point:

```powershell
scripts/install.ps1
```

What `scripts/install.ps1` does:

- checks for `cargo`, `node`, and `pnpm`
- runs `pnpm install`
- builds and packages Tablinq
- replaces the existing install under `%LOCALAPPDATA%\Tablinq\install`
- initializes the state root under `%LOCALAPPDATA%\Tablinq`
- registers the user-level native messaging host for Edge and Chrome
- updates the user `PATH` to include `%LOCALAPPDATA%\Tablinq\install\bin`
- removes legacy Startup-folder launchers and any existing Tablinq logon-time persistence
- does not create Startup-folder launchers, Run-key entries, or scheduled tasks by default
- starts the daemon and gateway by default and reports whether they are healthy
- prints a compact install summary plus the remaining manual browser steps

What still requires you:

1. If the summary says daemon or gateway need attention, run `gpt daemon start` or `gpt gateway start`.
2. Fully close Edge or Chrome.
3. Run `gpt browser start-debug`.
4. Load the unpacked extension from `%LOCALAPPDATA%\Tablinq\install\extension`.
5. If you are using Chrome attach-first, use the dedicated Chrome attach profile under `%LOCALAPPDATA%\Tablinq\chrome-attach-profile`.
6. Sign in manually at `https://chatgpt.com/` in that same browser profile.
7. Run `gpt browser attach`.
8. Verify with `gpt status`.

To enable Windows logon-time autostart explicitly later, rerun the installer with:

```powershell
scripts/install.ps1 -EnableAutoStart
```

To skip the default daemon and gateway launch, opt out explicitly:

```powershell
scripts/install.ps1 -NoLaunchAfterInstall
```

## Unix-like systems

Build and install:

```bash
./scripts/package.sh
./scripts/install.sh
```

Then:

1. If you plan to use Edge attach-first, load the unpacked extension from `~/.local/share/tablinq/install/extension` into the Edge profile you will attach to.
2. If the installer did not already bring it up successfully, start the daemon:
   - `~/.local/share/tablinq/install/bin/gpt daemon start`
3. If the installer did not already bring it up successfully, start the gateway:
   - `~/.local/share/tablinq/install/bin/gpt gateway start`
4. Fully close Chrome or Edge so the debug flags can take effect cleanly.
5. Start your normal browser in attachable mode:
   - `~/.local/share/tablinq/install/bin/gpt browser start-debug`
6. If you plan to use Chrome attach-first, load the unpacked extension into the dedicated Tablinq Chrome attach profile under the state root.
7. Sign in manually at `https://chatgpt.com/` in that same browser profile.
8. Attach and verify:
   - `~/.local/share/tablinq/install/bin/gpt browser attach`
   - `~/.local/share/tablinq/install/bin/gpt browser connection-info`
   - `~/.local/share/tablinq/install/bin/gpt status`
   - `~/.local/share/tablinq/install/bin/gpt doctor`
   - `~/.local/share/tablinq/install/bin/gpt capabilities`

The Unix installer prepares install and state directories, writes startup helper scripts, and attempts to start the daemon and gateway through the installed `gpt` binary. Native-host registration remains Windows-first.

## First-run checks

After install and attach, run:

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

- `<repo-root>`
- `%LOCALAPPDATA%\Tablinq`
- `%NVM_SYMLINK%`
- `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`
- `%NVM_SYMLINK%\node.exe`
- `<repo-root>\scripts`
- `%LOCALAPPDATA%\nvm`
- `%ProgramFiles%\PowerShell\7\pwsh.exe`
- `%ProgramFiles%\Google\Chrome\Application\chrome.exe`
- `%LOCALAPPDATA%\nvm\v20.19.3\node.exe`
- `%SystemRoot%\System32\cmd.exe`
- `<repo-root>\node_modules\.pnpm\@esbuild+win32-x64@0.27.7\node_modules\@esbuild\win32-x64\esbuild.exe`
- `<repo-root>\node_modules`

Adjust those paths for your own install directory, browser, and Node version.

## State layout

The state root contains:

- `config/config.toml`
- `logs/`
- `db/`
- `browser-profile/`
- `chrome-attach-profile/`
- `downloads/`
- `crash-bundles/`
- `gateway-uploads/`

`browser-profile/` still exists even though attach-first no longer uses a managed profile as the default ownership model. That directory remains useful for managed fallback and compatibility.

`chrome-attach-profile/` is the dedicated persistent real Chrome profile used by `gpt browser start-debug` in attach-first mode on current Chrome. Load the extension and sign in there when you are using Chrome attach-first.

If something goes wrong, see [troubleshooting.md](troubleshooting.md).
