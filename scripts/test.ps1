$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repoRoot

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if (Test-Path $cargoBin) {
  $env:PATH = "$cargoBin;$env:PATH"
}

function Invoke-Step {
  param(
    [string]$FilePath,
    [string[]]$ArgumentList = @()
  )

  Write-Host "> $FilePath $($ArgumentList -join ' ')"
  & $FilePath @ArgumentList
  if ($LASTEXITCODE -ne 0) {
    throw "Command failed: $FilePath $($ArgumentList -join ' ')"
  }
}

if (!(Test-Path (Join-Path $repoRoot "node_modules"))) {
  Invoke-Step pnpm @("install", "--frozen-lockfile")
}

$started = Get-Date

Invoke-Step cargo @("test", "--quiet", "-p", "gpt", "--locked")
Invoke-Step cargo @("test", "--quiet", "-p", "gptd", "--locked")
Invoke-Step cargo @("test", "--quiet", "-p", "shared-protocol", "--locked")
Invoke-Step cargo @("test", "--quiet", "-p", "tablinq-host", "--locked")

$gatewayTestFilters = @(
  "auth::",
  "errors::",
  "openai::",
  "routes::",
  "tests::capabilities_provider",
  "tests::chat_requests",
  "tests::conversation_inspection",
  "tests::failure_recovery_status",
  "tests::gateway_process",
  "tests::request_parsing",
  "tests::scheduler_process_control",
  "tests::uploads_artifacts"
)

foreach ($filter in $gatewayTestFilters) {
  Invoke-Step cargo @("test", "--quiet", "-p", "tablinq-gateway", "--locked", $filter)
}

Invoke-Step pnpm @("exec", "vitest", "run", "--reporter=dot")
$elapsed = (Get-Date) - $started
Write-Host ("All local tests passed in {0:mm\:ss}." -f $elapsed)
