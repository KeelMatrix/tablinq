# Tablinq Local Gateway Smoke Scripts

This folder contains local Windows smoke-test scripts for validating the Tablinq gateway against a real local browser-backed install.

These scripts are intended for developer validation after `scripts/install.ps1` has installed the current checkout and the installed `gpt.exe` matches the repo HEAD.

Run commands from the repo root unless a parameter explicitly points somewhere else.

## Folder contents

```text
scripts\smoke\local-gateway\run_step1_readiness.ps1
scripts\smoke\local-gateway\tablinq_step1_readiness.py

scripts\smoke\local-gateway\run_step1_2_diagnostics.ps1
scripts\smoke\local-gateway\tablinq_step1_2_diagnostics.py

scripts\smoke\local-gateway\run_step2_control_plane.ps1
scripts\smoke\local-gateway\tablinq_step2_control_plane.py

scripts\smoke\local-gateway\run_step3_chat_matrix.ps1
scripts\smoke\local-gateway\tablinq_step3_chat_matrix.py

scripts\smoke\local-gateway\run_step4_robustness.ps1
scripts\smoke\local-gateway\tablinq_step4_robustness.py

scripts\smoke\local-gateway\run_step5_response_shape.ps1
scripts\smoke\local-gateway\tablinq_step5_response_shape.py
```

## Prerequisites

Run from Windows PowerShell at the repo root.

Before using these scripts:

1. Build and install Tablinq from the repo root:

```powershell
.\scripts\install.ps1
```

2. Confirm the installed CLI is valid for the current checkout:

```powershell
git status
git log -1 --oneline
gpt version
```

The installed `gpt version` commit should match the repo HEAD, and the install summary should report the checkout as valid.

3. Create an `.env.local` file in `scripts\smoke\local-gateway`.

Expected API key variable names, in priority order:

```text
TABLINQ_GATEWAY_API_KEY
TABLINQ_API_KEY
GATEWAY_API_KEY
OPENAI_API_KEY
API_KEY
```

Optional base URL variable names:

```text
TABLINQ_GATEWAY_BASE_URL
GATEWAY_BASE_URL
OPENAI_BASE_URL
BASE_URL
```

Default gateway URL is:

```text
http://127.0.0.1:3847
```

## Safety rules

The scripts intentionally avoid deep research.

The scripts never use `tablinq-research`.

Prompt-submitting `POST /v1/chat/completions` requests are not blindly retried after transport failures, because the request may already have reached the gateway or browser.

Idempotent GET/status follow-up requests may be retried.

Generated logs redact the API key in cURL output.

## Run order

Run the scripts in this order from the repo root.

### Step 1: readiness

Starts the browser debug flow, attaches, and verifies basic gateway/browser readiness.

```powershell
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\smoke\local-gateway\run_step1_readiness.ps1
```

Pass condition:

- browser/debug process is reachable
- gateway is running
- browser is authenticated
- CDP attached
- extension connected
- target ChatGPT tab found
- final verdict is healthy

### Step 1.2: focused diagnostics

Runs after Step 1 while the browser remains open. Rechecks health/status convergence and transport stability on core browser/status routes.

```powershell
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\smoke\local-gateway\run_step1_2_diagnostics.ps1
```

Pass condition:

- no mixed `/health` state
- status routes agree on attached/authenticated browser state
- no final transport failures

### Step 2: control-plane route sweep

Exercises non-prompting gateway routes, route contracts, auth failures, bad IDs, browser attach/detach/restore, metadata, discovery, and stability loops.

```powershell
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\smoke\local-gateway\run_step2_control_plane.ps1
```

Pass condition:

- attach convergence passes
- `/health`, `/admin/status`, `/v1/browser/profile-info`, and `/v1/browser/connection-info` agree
- invalid route/auth/method cases return structured HTTP responses
- no final transport failures
- no semantic failures

### Step 3: chat matrix

Sends real non-research `/v1/chat/completions` prompts through several gateway modes and request shapes, including follow-up, search, thinking modes, multipart content, primary/branch metadata, auto-recover metadata, and invalid validation cases.

```powershell
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\smoke\local-gateway\run_step3_chat_matrix.ps1 -IncludeInvalidCases
```

Pass condition:

- all valid chat cases return 2xx
- invalid chat cases return structured HTTP errors
- no ambiguous chat POST transport failures
- no final transport failures
- no semantic failures
- auto-recover normal success remains `completed`, not `recovered`

### Step 4: robustness

Exercises negative/malformed route probes, upload/file flow, cancellation/race behavior, concurrency/ordering, final state checks, and cleanup detach/reattach.

Use `-NoTxt` when `.txt` uploads are temporarily broken at the provider level. In `-NoTxt` mode, the script uploads an extensionless file with `Content-Type: text/plain`.

```powershell
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\smoke\local-gateway\run_step4_robustness.ps1 -NoTxt
```

Pass condition:

- attach convergence passes
- negative and malformed cases return structured HTTP responses
- upload flow works
- chat with staged file ID works or returns an explainable structured error
- cancellation state is coherent
- concurrency/ordering is traceable and stable
- no ambiguous chat POST transport failures
- no final transport failures
- no semantic failures

### Step 5: response shape and pagination

Runs after Steps 1-4 so the gateway has recent requests, conversations, files, artifacts, and usage rows to inspect. The script is non-prompting: it only attaches, reads API state, validates response-shape contracts, and probes invalid query combinations.

```powershell
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\smoke\local-gateway\run_step5_response_shape.ps1
```

Pass condition:

- attach convergence passes
- list routes return coherent `limit`, `offset`, `count`, `has_more`, `view`, and `compact` metadata
- `/v1/usage` supports explicit durable pagination, including offsets past the historical 50-row boundary
- conversation message pagination supports `limit`, `offset`, and `order=desc`
- detail routes honor `view`, `include`, and compact snapshot behavior
- request evidence pagination works when a suitable request exists
- unsupported response-shape query params return structured HTTP errors
- no final transport failures
- no semantic failures

If a Step 5 ID-specific probe has no suitable existing object, the script records a warning and skips that subcase instead of fabricating state.

## Output files

Each script writes a timestamped markdown log in `scripts\smoke\local-gateway`. Logs include:

- short notes describing each probe
- raw redacted cURL requests
- raw response headers
- raw response bodies
- final summary JSON
- final verdict

Example generated files:

```text
scripts\smoke\local-gateway\tablinq-step1-readiness-v2-YYYYMMDD-HHMMSS.md
scripts\smoke\local-gateway\tablinq-step1-2-diagnostics-YYYYMMDD-HHMMSS.md
scripts\smoke\local-gateway\tablinq-step2-control-plane-v2-YYYYMMDD-HHMMSS.md
scripts\smoke\local-gateway\tablinq-step3-chat-matrix-v5-YYYYMMDD-HHMMSS.md
scripts\smoke\local-gateway\tablinq-step4-robustness-YYYYMMDD-HHMMSS.md
scripts\smoke\local-gateway\tablinq-step5-response-shape-YYYYMMDD-HHMMSS.md
```

Generated logs and temporary upload fixtures should not normally be committed.

## Recommended cleanup rule

Commit only these files:

```text
scripts\smoke\local-gateway\README.md
scripts\smoke\local-gateway\run_step1_readiness.ps1
scripts\smoke\local-gateway\tablinq_step1_readiness.py
scripts\smoke\local-gateway\run_step1_2_diagnostics.ps1
scripts\smoke\local-gateway\tablinq_step1_2_diagnostics.py
scripts\smoke\local-gateway\run_step2_control_plane.ps1
scripts\smoke\local-gateway\tablinq_step2_control_plane.py
scripts\smoke\local-gateway\run_step3_chat_matrix.ps1
scripts\smoke\local-gateway\tablinq_step3_chat_matrix.py
scripts\smoke\local-gateway\run_step4_robustness.ps1
scripts\smoke\local-gateway\tablinq_step4_robustness.py
scripts\smoke\local-gateway\run_step5_response_shape.ps1
scripts\smoke\local-gateway\tablinq_step5_response_shape.py
```

Do not commit:

```text
scripts\smoke\local-gateway\tablinq-step*.md
scripts\smoke\local-gateway\*.stdout.log
scripts\smoke\local-gateway\*.stderr.log
scripts\smoke\local-gateway\__pycache__\
scripts\smoke\local-gateway\step4-upload-field-note-*
scripts\smoke\local-gateway\.env.local
```

## Suggested final validation before committing

From the repo root:

```powershell
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\smoke\local-gateway\run_step1_readiness.ps1
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\smoke\local-gateway\run_step1_2_diagnostics.ps1
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\smoke\local-gateway\run_step2_control_plane.ps1
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\smoke\local-gateway\run_step3_chat_matrix.ps1 -IncludeInvalidCases
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\smoke\local-gateway\run_step4_robustness.ps1 -NoTxt
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\smoke\local-gateway\run_step5_response_shape.ps1
```

All final verdicts should be `healthy`.
