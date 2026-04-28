# Troubleshooting

This guide follows the current attach-first operating model.

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

## Why attach-first is the right model

Tablinq does not use a Playwright-owned managed login flow as the normal path anymore. That approach proved to be the wrong default because Cloudflare and anti-bot protections at `chatgpt.com` made automation-owned browser startup and login unreliable.

The supported model is:

- attach target browser profile
- manual sign-in
- local debug endpoint
- Tablinq attach
- extension bridge on the real tab

For current Chrome attach-first, the attach target is the dedicated `chrome-attach-profile` under the Tablinq state root. For Edge attach-first, it is normally the Edge profile started by `gpt browser start-debug`.

Playwright is still used internally for attach, navigation assistance, screenshots, uploads, downloads, and recovery. It is no longer the default owner of the ChatGPT login session.

## Attach endpoint not reachable

Symptoms:

- `gpt doctor` shows attach availability problems
- `gpt browser attach` fails immediately
- `gpt browser connection-info` points at a debug endpoint that is not live

Actions:

- fully close the configured browser before retrying
- run `gpt browser start-debug`
- run `gpt browser connection-info`
- confirm the reported `debug_endpoint`, `browser_path`, and target URL match what you expect
- if no browser executable is detected, set `browser_path` in `config/config.toml`

## Extension missing or disconnected

Symptoms:

- `gpt doctor` shows extension connectivity is false
- `gpt status` reports that the browser is attached but the extension is disconnected
- capabilities stay empty even though the tab is visible

Actions:

- confirm the unpacked Tablinq extension is installed in the same browser profile Tablinq is attaching to
- reload or reinstall the extension from the installed `extension` directory
- keep only one signed-in ChatGPT tab open while reconnecting if you suspect the wrong tab was selected
- rerun `gpt browser attach`

## Signed out or blocked on login or challenge

Symptoms:

- `gpt status` shows `authenticated = false`
- provider requests fail with `auth_required`
- the current ChatGPT tab is on login, challenge, or captcha UI

Actions:

- sign in manually at `https://chatgpt.com/` in the attached browser profile
- clear any login, challenge, or captcha page before retrying
- rerun `gpt browser attach`
- if Tablinq attached to the wrong tab, run `gpt browser open`

## Target tab missing or wrong tab selected

Symptoms:

- `gpt doctor` shows the target tab is missing
- multiple ChatGPT tabs are open and Tablinq keeps choosing the wrong one

Actions:

- run `gpt browser open` to create or focus the configured target tab
- keep only the signed-in ChatGPT tab with the working Tablinq extension open while retrying attach
- rerun `gpt browser attach`

## Interrupted or recovered requests

Symptoms:

- a request reports `interrupted`
- the browser reloaded, disconnected, or the local client dropped mid-stream
- a later request or status view shows `recovered`

What that means:

- `interrupted` means the local stream broke, not necessarily that upstream browser work stopped
- `recovered` means Tablinq reattached, reconciled the live browser conversation, and found a safe matching result

Actions:

- inspect the request with `gpt jobs show <job-id>` or `GET /v1/requests/{request_id}`
- do not blindly resubmit the same prompt
- if needed, rerun `gpt browser attach` and `gpt browser open`

## Provider page says "Something went wrong while generating the response"

Symptoms:

- the ChatGPT page shows `Something went wrong while generating the response...`
- the CLI or gateway returns `provider_generation_failed_nonretryable`

What that means:

- Tablinq treats this as a provider failure where the page Retry button is intentionally disabled
- the default recovery path is reload, fresh conversation, and one replay with preserved compact context
- the replay does not depend on the provider preserving the old thread across reload

Current context scope for this recovery:

- original user request
- request options and request-scoped file references Tablinq already knows about
- prior conversation id
- a durable recovery manifest synthesized from prior completed turns
- an optional recovery-manifest attachment uploaded into the fresh conversation
- a bounded replay file set that can include recent durable context files and generated artifacts when they still exist locally

Current limitation:

- this is not a full transcript restore
- provider-only context and old provider-hosted artifacts are only recoverable if they already exist in Tablinq's own durable state
- replayed historical files are intentionally bounded, so very large or very attachment-heavy threads may still lose some provider-side nuance

Actions:

- inspect `gpt jobs show <job-id>` or `GET /v1/requests/{request_id}` for recovery metadata
- inspect the linked failure bundle if the replay also fails
- if your workflow depends on deep prior-thread context, prefer explicit attachments or external durable context instead of relying on implicit browser-thread memory alone

## Native messaging does not connect

Symptoms:

- the extension never connects
- page probing fails even though the browser and daemon are running

Actions:

- rerun the installer so native-host registration is refreshed
- confirm the unpacked extension path matches the installed `extension` directory
- inspect the local logs under the Tablinq state root

## Managed fallback only for edge cases

Use managed fallback only when attach mode cannot be enabled on this machine or browser profile.

Actions:

- set `browser_connection_mode = "managed_only"` in `config/config.toml`
- run `gpt browser open`
- use `gpt browser reset-profile` only for managed fallback profile recovery

## Browser adapter fails after install

Symptoms:

- the daemon starts but browser operations fail immediately
- installed behavior differs from repo-checkout behavior

Actions:

- run `gpt version --json`
- verify the packaged browser adapter output exists under the installed package layout
- remember that the daemon prefers packaged `service.js` output and falls back to repo `service.ts` only in a developer checkout

## Gateway unhealthy or not ready

Symptoms:

- `gpt gateway status` is unhealthy
- `/health` or `/ready` fails
- provider clients time out before work starts

Actions:

- run `gpt gateway status`
- check `GET /health`, `GET /ready`, and `GET /v1/provider`
- inspect `gateway.log`
- restart in order:
  - `gpt gateway stop`
  - `gpt daemon stop`
  - `gpt daemon start`
  - `gpt gateway start`

## Antivirus blocking local tools or browser startup

Symptoms:

- packaging, tests, or install steps fail with `Access is denied`
- the browser does not start when invoked by Tablinq
- `node.exe`, `pwsh.exe`, `chrome.exe`, `cmd.exe`, or `esbuild.exe` show as blocked or quarantined

Actions:

- restore or allow the blocked executable in your antivirus notifications or quarantine first
- add the relevant paths to antivirus exceptions
- if your antivirus has a separate behavioral or advanced threat module, add the same executables there too

Example Windows paths that may need exceptions:

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

Adjust those paths for your own machine.

## Clearing local state

Use the smallest reset that matches the problem:

- delete `downloads/`
- delete `crash-bundles/`
- delete `gateway-uploads/`

`gpt browser reset-profile` is a managed fallback recovery tool only.

Do not delete the full state root unless you want to discard local history and config.

## Selector-pack changes

If the live site drifts:

- edit `selector-packs/v1/chatgpt-default.json` before packaging, or replace the installed selector-pack copy
- restart `gptd` after changing selector-pack data

For explicit real-site checks, use [live-smoke-checklist.md](live-smoke-checklist.md).
