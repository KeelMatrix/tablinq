param(
  [string[]]$Job = @("quick")
)

$ErrorActionPreference = "Stop"
$script:PnpmInstallComplete = $false

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

function Ensure-PnpmInstall {
  param([string]$RepoRoot)

  if ($script:PnpmInstallComplete) {
    return
  }

  Invoke-Step pnpm @("install", "--frozen-lockfile") $RepoRoot
  $script:PnpmInstallComplete = $true
}

function Run-RustJob {
  param([string]$RepoRoot)

  Write-Host "==> Rust checks"
  Run-RustFormatJob -RepoRoot $RepoRoot
  Run-RustClippyJob -RepoRoot $RepoRoot
  Run-RustTestJob -RepoRoot $RepoRoot
}

function Run-RustFormatJob {
  param([string]$RepoRoot)

  Write-Host "==> Rust format"
  Invoke-Step cargo @("fmt", "--all", "--check") $RepoRoot
}

function Run-RustClippyJob {
  param([string]$RepoRoot)

  Write-Host "==> Rust clippy"
  Invoke-Step cargo @("clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings") $RepoRoot
}

function Run-RustTestJob {
  param([string]$RepoRoot)

  Write-Host "==> Rust tests"
  $previousSkip = $env:TABLINQ_SKIP_BROWSER_INTEGRATION_TESTS
  $previousPackageSmokeSkip = $env:TABLINQ_SKIP_PACKAGE_SMOKE_TESTS
  $env:TABLINQ_SKIP_BROWSER_INTEGRATION_TESTS = "1"
  $env:TABLINQ_SKIP_PACKAGE_SMOKE_TESTS = "1"
  try {
    Invoke-Step cargo @("test", "--workspace", "--locked") $RepoRoot
  }
  finally {
    if ($null -eq $previousSkip) {
      Remove-Item Env:TABLINQ_SKIP_BROWSER_INTEGRATION_TESTS -ErrorAction SilentlyContinue
    }
    else {
      $env:TABLINQ_SKIP_BROWSER_INTEGRATION_TESTS = $previousSkip
    }

    if ($null -eq $previousPackageSmokeSkip) {
      Remove-Item Env:TABLINQ_SKIP_PACKAGE_SMOKE_TESTS -ErrorAction SilentlyContinue
    }
    else {
      $env:TABLINQ_SKIP_PACKAGE_SMOKE_TESTS = $previousPackageSmokeSkip
    }
  }
}

function Run-RustBrowserRecoveryJob {
  param([string]$RepoRoot)

  Write-Host "==> Rust browser recovery tests"
  Ensure-PnpmInstall $RepoRoot
  $previousSkip = $env:TABLINQ_SKIP_BROWSER_INTEGRATION_TESTS
  try {
    Remove-Item Env:TABLINQ_SKIP_BROWSER_INTEGRATION_TESTS -ErrorAction SilentlyContinue
    Invoke-Step cargo @(
      "test",
      "-p",
      "gptd",
      "--test",
      "core_service_recovery",
      "--locked",
      "--",
      "--nocapture"
    ) $RepoRoot
  }
  finally {
    if ($null -eq $previousSkip) {
      Remove-Item Env:TABLINQ_SKIP_BROWSER_INTEGRATION_TESTS -ErrorAction SilentlyContinue
    }
    else {
      $env:TABLINQ_SKIP_BROWSER_INTEGRATION_TESTS = $previousSkip
    }
  }
}

function Run-TypeScriptJob {
  param([string]$RepoRoot)

  Write-Host "==> TypeScript core checks"
  Ensure-PnpmInstall $RepoRoot
  Invoke-Step pnpm @("format") $RepoRoot
  Invoke-Step pnpm @("lint") $RepoRoot
  Invoke-Step pnpm @("build") $RepoRoot
  Invoke-Step pnpm @("run", "test:ts:fast") $RepoRoot
}

function Run-TypeScriptTestJob {
  param([string]$RepoRoot)

  Write-Host "==> TypeScript full tests"
  Ensure-PnpmInstall $RepoRoot
  Invoke-Step pnpm @("exec", "vitest", "run", "--maxWorkers=4") $RepoRoot
}

function Run-TypeScriptTestShardJob {
  param([string]$RepoRoot, [int]$Shard)

  Write-Host "==> TypeScript test shard $Shard/10"
  Ensure-PnpmInstall $RepoRoot
  Invoke-Step pnpm @("exec", "vitest", "run", "--shard=$Shard/10", "--reporter=dot") $RepoRoot
}

function Run-TypeScriptFormatJob {
  param([string]$RepoRoot)

  Write-Host "==> TypeScript format"
  Ensure-PnpmInstall $RepoRoot
  Invoke-Step pnpm @("format") $RepoRoot
}

function Run-BrowserAdapterAskJob {
  param([string]$RepoRoot)

  Write-Host "==> Browser adapter ask"
  Ensure-PnpmInstall $RepoRoot
  Invoke-Step pnpm @("run", "test:browser-adapter:ask") $RepoRoot
}

function Run-BrowserAdapterFoundationJob {
  param([string]$RepoRoot)

  Write-Host "==> Browser adapter foundation"
  Ensure-PnpmInstall $RepoRoot
  Invoke-Step pnpm @("run", "test:browser-adapter:foundation") $RepoRoot
}

function Run-BrowserAdapterResearchJob {
  param([string]$RepoRoot)

  Write-Host "==> Browser adapter research"
  Ensure-PnpmInstall $RepoRoot
  Invoke-Step pnpm @("run", "test:browser-adapter:research") $RepoRoot
}

function Run-BrowserAdapterRecoveryJob {
  param([string]$RepoRoot)

  Write-Host "==> Browser adapter recovery"
  Ensure-PnpmInstall $RepoRoot
  Invoke-Step pnpm @("run", "test:browser-adapter:recovery") $RepoRoot
}

function Run-PackageSmokeJob {
  param([string]$RepoRoot, [string]$ScriptRoot)

  Write-Host "==> Package smoke"
  Ensure-PnpmInstall $RepoRoot
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
$env:CARGO_PROFILE_DEV_DEBUG = "0"
$env:CARGO_PROFILE_TEST_DEBUG = "0"
$env:RUST_BACKTRACE = "1"
$env:CI = "true"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$validJobs = @(
  "quick",
  "all",
  "full",
  "rust",
  "rust-fmt",
  "rust-clippy",
  "rust-test",
  "rust-browser-recovery",
  "typescript",
  "typescript-test",
  "typescript-test-shard-1",
  "typescript-test-shard-2",
  "typescript-test-shard-3",
  "typescript-test-shard-4",
  "typescript-test-shard-5",
  "typescript-test-shard-6",
  "typescript-test-shard-7",
  "typescript-test-shard-8",
  "typescript-test-shard-9",
  "typescript-test-shard-10",
  "typescript-format",
  "browser-adapter-foundation",
  "browser-adapter-ask",
  "browser-adapter-research",
  "browser-adapter-recovery",
  "browser-adapter",
  "package-smoke"
)
$requestedJobs = @(
  foreach ($entry in $Job) {
    foreach ($name in ($entry -split ",")) {
      $trimmed = $name.Trim()
      if ($trimmed) {
        $trimmed
      }
    }
  }
)

foreach ($name in $requestedJobs) {
  if ($validJobs -notcontains $name) {
    throw "Unknown CI job '$name'. Valid jobs: $($validJobs -join ', ')"
  }
}

$jobs = foreach ($name in $requestedJobs) {
  switch ($name) {
    "quick" {
      "rust-fmt"
      "typescript"
    }
    "all" {
      "rust"
      "rust-browser-recovery"
      "typescript"
      "typescript-test"
      "package-smoke"
    }
    "full" {
      "rust"
      "rust-browser-recovery"
      "typescript"
      "typescript-test"
      "package-smoke"
    }
    "browser-adapter" {
      "browser-adapter-foundation"
      "browser-adapter-ask"
      "browser-adapter-research"
      "browser-adapter-recovery"
    }
    default { $name }
  }
}

foreach ($name in $jobs) {
  switch ($name) {
    "rust" { Run-RustJob -RepoRoot $repoRoot }
    "rust-fmt" { Run-RustFormatJob -RepoRoot $repoRoot }
    "rust-clippy" { Run-RustClippyJob -RepoRoot $repoRoot }
    "rust-test" { Run-RustTestJob -RepoRoot $repoRoot }
    "rust-browser-recovery" { Run-RustBrowserRecoveryJob -RepoRoot $repoRoot }
    "typescript" { Run-TypeScriptJob -RepoRoot $repoRoot }
    "typescript-test" { Run-TypeScriptTestJob -RepoRoot $repoRoot }
    "typescript-test-shard-1" { Run-TypeScriptTestShardJob -RepoRoot $repoRoot -Shard 1 }
    "typescript-test-shard-2" { Run-TypeScriptTestShardJob -RepoRoot $repoRoot -Shard 2 }
    "typescript-test-shard-3" { Run-TypeScriptTestShardJob -RepoRoot $repoRoot -Shard 3 }
    "typescript-test-shard-4" { Run-TypeScriptTestShardJob -RepoRoot $repoRoot -Shard 4 }
    "typescript-test-shard-5" { Run-TypeScriptTestShardJob -RepoRoot $repoRoot -Shard 5 }
    "typescript-test-shard-6" { Run-TypeScriptTestShardJob -RepoRoot $repoRoot -Shard 6 }
    "typescript-test-shard-7" { Run-TypeScriptTestShardJob -RepoRoot $repoRoot -Shard 7 }
    "typescript-test-shard-8" { Run-TypeScriptTestShardJob -RepoRoot $repoRoot -Shard 8 }
    "typescript-test-shard-9" { Run-TypeScriptTestShardJob -RepoRoot $repoRoot -Shard 9 }
    "typescript-test-shard-10" { Run-TypeScriptTestShardJob -RepoRoot $repoRoot -Shard 10 }
    "typescript-format" { Run-TypeScriptFormatJob -RepoRoot $repoRoot }
    "browser-adapter-foundation" { Run-BrowserAdapterFoundationJob -RepoRoot $repoRoot }
    "browser-adapter-ask" { Run-BrowserAdapterAskJob -RepoRoot $repoRoot }
    "browser-adapter-research" { Run-BrowserAdapterResearchJob -RepoRoot $repoRoot }
    "browser-adapter-recovery" { Run-BrowserAdapterRecoveryJob -RepoRoot $repoRoot }
    "package-smoke" { Run-PackageSmokeJob -RepoRoot $repoRoot -ScriptRoot $PSScriptRoot }
  }
}
