# Troubleshooting

This guide focuses on the common operational failures for a browser-driven local provider.

## Start here

Before changing anything, gather:

- `gpt status`
- `gpt doctor`
- `gpt browser connection-info`
- `gpt capabilities`
- the failing CLI command or gateway request
- the latest `request_id` or `job_id`, if available

If a failure bundle was written, keep the latest bundle under `crash-bundles/`.

Use this recovery order in attach-first mode:

1. check the local debug endpoint
2. check extension installation and connection
3. check the signed-in ChatGPT tab
4. reattach or open the target tab
5. only then consider managed fallback

## Attach Endpoint Not Reachable

Symptoms:

- `gpt doctor` shows `attach_unavailable`
- `gpt browser attach` fails immediately
- `gpt browser connection-info` points at a debug endpoint that is not live

Actions:

- fully close Chrome or Edge before retrying
- run `gpt browser start-debug`
- run `gpt browser connection-info` and confirm:
  - `debug_endpoint`
  - `browser_path`
  - `target_url`
- if no browser executable is detected, set `browser_path` in `%LOCALAPPDATA%\Tablinq\config\config.toml`

## Extension Missing Or Disconnected

Symptoms:

- `gpt doctor` shows extension connectivity is false
- `gpt status` reports `attached_extension_disconnected`
- the browser is attached but capabilities stay empty

Actions:

- confirm the unpacked Tablinq extension is installed in the same normal browser profile Tablinq is attaching to
- reload the extension or reinstall it from the installed `extension` directory
- keep only one ChatGPT tab open while reconnecting if you suspect Tablinq attached to the wrong tab
- rerun `gpt browser attach`

## Signed Out Or Blocked On Login Or Challenge

Symptoms:

- `gpt status` shows `authenticated = false`
- provider requests fail with `auth_required`
- the current ChatGPT tab is on login, challenge, or captcha UI

Actions:

- sign in manually at `chatgpt.com` in your normal browser
- clear any login, challenge, or captcha page before retrying
- rerun `gpt browser attach`
- if the browser is attached but Tablinq is still looking at the wrong tab, run `gpt browser open`

## Target Tab Missing Or Wrong Tab Selected

Symptoms:

- `gpt doctor` shows `target_tab_found = false`
- `gpt status` reports `target_tab_missing`
- multiple ChatGPT tabs are open and Tablinq keeps choosing the wrong one

Actions:

- run `gpt browser open` to create or focus the configured ChatGPT tab
- leave only the signed-in ChatGPT tab with the working Tablinq extension open while retrying attach
- rerun `gpt browser attach`

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
- if needed, rerun `gpt browser attach` and `gpt browser open`

## Native messaging does not connect

Symptoms:

- the extension never connects
- page probing fails even though the browser launches

Actions:

- re-run `scripts/install.ps1` to refresh registry keys and native-host manifests
- confirm the unpacked extension path matches `%LOCALAPPDATA%\Tablinq\install\extension`
- check `%LOCALAPPDATA%\Tablinq\logs\native-host.log` if present

## Managed Fallback Only For Edge Cases

Use managed fallback only when attach mode cannot be enabled on this machine or browser profile.

Actions:

- set `browser_connection_mode = "managed_only"` in `config/config.toml`
- run `gpt browser open`
- use `gpt browser reset-profile` only for managed fallback profile recovery, not as the normal attach-first fix

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

- delete `%LOCALAPPDATA%\Tablinq\downloads`
  - removes downloaded artifacts
- delete `%LOCALAPPDATA%\Tablinq\crash-bundles`
  - clears saved failure bundles
- delete `%LOCALAPPDATA%\Tablinq\gateway-uploads`
  - clears staged provider uploads

`gpt browser reset-profile` is a managed fallback recovery tool only.

Do not delete the full state root unless you really want to discard local history and config.

## Selector-pack changes

If the live site drifts:

- edit `selector-packs/v1/chatgpt-default.json` before packaging, or replace the installed copy under `%LOCALAPPDATA%\Tablinq\install\selector-packs`
- restart `gptd` after changing selector-pack data so the next probe reloads it

For explicit real-site checks, use [live-smoke-checklist.md](live-smoke-checklist.md).
