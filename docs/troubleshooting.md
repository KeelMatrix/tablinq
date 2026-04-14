# Troubleshooting

This guide focuses on the common operational failures for a browser-driven local provider.

## Start here

Before changing anything, gather:

- `gpt status`
- `gpt doctor`
- `gpt capabilities`
- the failing CLI command or gateway request
- the latest `request_id` or `job_id`, if available

If a failure bundle was written, keep the latest bundle under `crash-bundles/`.

## Browser not detected

Symptoms:

- `gpt doctor` reports no browser
- `gpt browser open` fails before the page loads

Actions:

- run `gpt doctor` and inspect `browser_detection`
- on Windows, verify `msedge.exe` or `chrome.exe` exists in a standard install path
- if needed, set `browser_path` in `%LOCALAPPDATA%\Tablinq\config\config.toml`

## Not authenticated or wrong account

Symptoms:

- `gpt status` shows `authenticated = false`
- provider requests fail with `auth_required`
- the managed browser is signed into the wrong ChatGPT account

Actions:

- run `gpt status` or `gpt doctor`
- use one of:
  - `gpt browser logout`
  - `gpt browser switch-account`
- if the profile is corrupted or wedged:
  - `gpt browser reset-profile`

## Request interrupted or recovered

Symptoms:

- a request reports `interrupted`
- the client disconnected or the browser reloaded mid-stream
- a later request shows `recovered`

What this means:

- `interrupted` means the local stream broke, not necessarily that the upstream browser work stopped
- `recovered` means Tablinq reattached, reconciled the live conversation, and found a safe matching result

Actions:

- inspect the request through `gpt jobs show <job-id>` or `GET /v1/requests/{request_id}`
- avoid resubmitting the same prompt blindly
- if needed, reopen the managed browser with `gpt browser open`

## Native messaging does not connect

Symptoms:

- the extension never connects
- page probing fails even though the browser launches

Actions:

- re-run `scripts/install.ps1` to refresh registry keys and native-host manifests
- confirm the unpacked extension path matches `%LOCALAPPDATA%\Tablinq\install\extension`
- check `%LOCALAPPDATA%\Tablinq\logs\native-host.log` if present

## Browser adapter fails after install

Symptoms:

- the daemon starts but browser operations fail immediately
- installed behavior differs from repo checkout behavior

Actions:

- run `gpt version --json` and confirm the package came from the expected commit
- verify `%LOCALAPPDATA%\Tablinq\install\browser-adapter\src\service.js` exists
- remember that the daemon prefers packaged `service.js` output and falls back to repo `service.ts` only in a developer checkout

## Gateway unhealthy or not ready

Symptoms:

- `gpt gateway status` is unhealthy
- `/health` or `/ready` fails
- provider clients time out before work starts

Actions:

- run `gpt gateway status`
- check `GET /health`, `GET /ready`, and `GET /v1/provider`
- inspect `%LOCALAPPDATA%\Tablinq\logs\gateway.log`
- restart in order:
  - `gpt gateway stop`
  - `gpt daemon stop`
  - `gpt daemon start`
  - `gpt gateway start`

## Clearing local state

Use the smallest reset that matches the problem:

- `gpt browser reset-profile`
  - recreates the managed browser profile
- delete `%LOCALAPPDATA%\Tablinq\downloads`
  - removes downloaded artifacts
- delete `%LOCALAPPDATA%\Tablinq\crash-bundles`
  - clears saved failure bundles
- delete `%LOCALAPPDATA%\Tablinq\gateway-uploads`
  - clears staged provider uploads

Do not delete the full state root unless you really want to discard local history and config.

## Selector-pack changes

If the live site drifts:

- edit `selector-packs/v1/chatgpt-default.json` before packaging, or replace the installed copy under `%LOCALAPPDATA%\Tablinq\install\selector-packs`
- restart `gptd` after changing selector-pack data so the next probe reloads it

For explicit real-site checks, use [live-smoke-checklist.md](live-smoke-checklist.md).
