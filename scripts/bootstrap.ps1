$ErrorActionPreference = "Stop"

$cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
if (Test-Path $cargoBin) {
  $env:PATH = "$cargoBin;$env:PATH"
}

Write-Host "Node: $(node --version)"
Write-Host "pnpm: $(pnpm --version)"
Write-Host "cargo: $(cargo --version)"
Write-Host "Install: scripts/install.ps1"
Write-Host "Optional package-only script: scripts/package.ps1"
