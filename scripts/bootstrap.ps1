$ErrorActionPreference = "Stop"

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if (Test-Path $cargoBin) {
  $env:PATH = "$cargoBin;$env:PATH"
}

Write-Host "Node: $(node --version)"
Write-Host "pnpm: $(pnpm --version)"
Write-Host "cargo: $(cargo --version)"
