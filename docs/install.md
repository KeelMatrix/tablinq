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
- removes any existing scheduled-task or Run-key logon-time persistence
- does not create Run-key entries or scheduled tasks by default
- starts the daemon and gateway by default and reports whether they are healthy
- prints a compact install summary, including the installed CLI build, the current-shell `gpt` target, repo-HEAD mismatch warnings for contributor checkouts, and the remaining manual browser steps

If you are working from a repo checkout, prefer `cargo run -p gpt -- ...` while validating local code changes. Plain `gpt` runs the installed binary on `PATH`, which can remain on an older build until install completes and a new shell picks up the updated path state.

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
5. Start an attachable browser session:
   - `~/.local/share/tablinq/install/bin/gpt browser start-debug`
6. If you plan to use Chrome attach-first, load the unpacked extension into the dedicated Tablinq Chrome attach profile under the state root.
7. Sign in manually at `https://chatgpt.com/` in that same attach target profile.
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
- `gpt ask "Draft a two-line release note for faster startup."`

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

Tablinq no longer exposes a browser profile reset command. If a managed fallback profile is wedged, close managed browser instances before retrying `gpt browser open`.

## Windows Install Failures

See [troubleshooting](troubleshooting.md#antivirus-blocking-local-tools-or-browser-startup) for symptoms and exception candidates.

## State layout

The state root contains:

- `config/config.toml`
- `logs/`
- `db/`
- `browser-profile/`
- `chrome-attach-profile/`
- `downloads/`
- `crash-bundles/`
- `recovery-manifests/`
- `gateway-uploads/`
- `daemon.lock`
- `gateway.lock`
- `gateway-status.json`

`browser-profile/` still exists even though attach-first no longer uses a managed profile as the default ownership model. That directory remains useful for managed fallback and compatibility.

`chrome-attach-profile/` is the dedicated persistent real Chrome profile used by `gpt browser start-debug` in attach-first mode on current Chrome. Load the extension and sign in there when you are using Chrome attach-first.

If something goes wrong, see [troubleshooting.md](troubleshooting.md).
