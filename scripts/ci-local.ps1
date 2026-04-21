param(
  [ValidateSet("all", "rust", "typescript", "package-smoke")]
  [string[]]$Job = @("all")
)

$ErrorActionPreference = "Stop"

function Add-CargoBinToPath {
  $cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
  if (Test-Path $cargoBin) {
    $env:PATH = "$cargoBin;$env:PATH"
  }
}

function Invoke-Step {
  param(
    [string]$FilePath,
    [string[]]$ArgumentList = @(),
    [string]$WorkingDirectory
  )

  Write-Host "> $FilePath $($ArgumentList -join ' ')"
  if ($WorkingDirectory) {
    Push-Location $WorkingDirectory
  }

  try {
    & $FilePath @ArgumentList
    if ($LASTEXITCODE -ne 0) {
      throw "Command failed: $FilePath $($ArgumentList -join ' ')"
    }
  }
  finally {
    if ($WorkingDirectory) {
      Pop-Location
    }
  }
}

function Run-RustJob {
  param([string]$RepoRoot)

  Write-Host "==> Rust checks"
  Invoke-Step cargo @("fmt", "--all", "--check") $RepoRoot
  Invoke-Step cargo @("clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings") $RepoRoot
  Invoke-Step cargo @("test", "--workspace", "--locked") $RepoRoot
}

function Run-TypeScriptJob {
  param([string]$RepoRoot)

  Write-Host "==> TypeScript checks"
  Invoke-Step pnpm @("install", "--frozen-lockfile") $RepoRoot
  Invoke-Step pnpm @("format") $RepoRoot
  Invoke-Step pnpm @("lint") $RepoRoot
  Invoke-Step pnpm @("build") $RepoRoot
  Invoke-Step pnpm @("test") $RepoRoot
}

function Run-PackageSmokeJob {
  param([string]$RepoRoot, [string]$ScriptRoot)

  Write-Host "==> Package smoke"
  Invoke-Step pnpm @("install", "--frozen-lockfile") $RepoRoot
  Invoke-Step powershell @(
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    (Join-Path $ScriptRoot "package.ps1"),
    "-Profile",
    "dev"
  ) $RepoRoot

  $packageRoot = Join-Path $RepoRoot "dist\package\tablinq"
  $previousPackageRoot = $env:TABLINQ_PACKAGE_ROOT
  $env:TABLINQ_PACKAGE_ROOT = $packageRoot
  try {
    Invoke-Step cargo @(
      "test",
      "-p",
      "gpt",
      "--test",
      "production_smoke",
      "windows_packaging_and_install_smoke_cover_packaged_layout",
      "--",
      "--nocapture"
    ) $RepoRoot
  }
  finally {
    if ($null -eq $previousPackageRoot) {
      Remove-Item Env:TABLINQ_PACKAGE_ROOT -ErrorAction SilentlyContinue
    }
    else {
      $env:TABLINQ_PACKAGE_ROOT = $previousPackageRoot
    }
  }
}

Add-CargoBinToPath

$env:CARGO_TERM_COLOR = "always"
$env:RUST_BACKTRACE = "1"
$env:CI = "true"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$jobs = if ($Job -contains "all") {
  @("rust", "typescript", "package-smoke")
}
else {
  $Job
}

foreach ($name in $jobs) {
  switch ($name) {
    "rust" { Run-RustJob -RepoRoot $repoRoot }
    "typescript" { Run-TypeScriptJob -RepoRoot $repoRoot }
    "package-smoke" { Run-PackageSmokeJob -RepoRoot $repoRoot -ScriptRoot $PSScriptRoot }
  }
}
