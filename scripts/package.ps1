param(
  [string]$Profile = "release",
  [string]$OutputDir = (Join-Path $PSScriptRoot "..\\dist\\package"),
  [string]$TargetDir = $env:CARGO_TARGET_DIR
)

$ErrorActionPreference = "Stop"

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

function Resolve-TargetProfileDir {
  param([string]$Profile)

  switch ($Profile) {
    "dev" { return "debug" }
    default { return $Profile }
  }
}

function Get-ExtensionIdFromKey {
  param([string]$ManifestPath)

  $manifest = Get-Content $ManifestPath | ConvertFrom-Json
  if (-not $manifest.key) {
    throw "Extension manifest is missing a stable key: $ManifestPath"
  }

  $publicKeyBytes = [Convert]::FromBase64String($manifest.key)
  $sha256 = [System.Security.Cryptography.SHA256]::Create()
  try {
    $hash = $sha256.ComputeHash($publicKeyBytes)
  }
  finally {
    $sha256.Dispose()
  }
  $alphabet = "abcdefghijklmnop".ToCharArray()
  $builder = [System.Text.StringBuilder]::new()

  for ($index = 0; $index -lt 16; $index += 1) {
    $byte = $hash[$index]
    [void]$builder.Append($alphabet[($byte -shr 4) -band 0x0f])
    [void]$builder.Append($alphabet[$byte -band 0x0f])
  }

  $builder.ToString()
}

function Copy-DirectoryContents {
  param(
    [string]$Source,
    [string]$Destination
  )

  New-Item -ItemType Directory -Force -Path $Destination | Out-Null
  Copy-Item -Path (Join-Path $Source "*") -Destination $Destination -Recurse -Force
}

function Get-RelativePath {
  param(
    [string]$BasePath,
    [string]$TargetPath
  )

  Push-Location $BasePath
  try {
    (Resolve-Path -Relative $TargetPath)
  }
  finally {
    Pop-Location
  }
}

function Get-WorkspaceHeadCommit {
  param([string]$RepoRoot)

  $output = & git -C $RepoRoot rev-parse HEAD 2>$null
  if ($LASTEXITCODE -ne 0) {
    throw "Unable to determine repo HEAD for package provenance."
  }

  $value = ($output | Out-String).Trim()
  if (-not $value) {
    throw "Unable to determine repo HEAD for package provenance."
  }

  $value
}

function Get-WorkspaceDirtyState {
  param([string]$RepoRoot)

  $output = & git -C $RepoRoot status --porcelain=v1 --untracked-files=no --ignore-submodules 2>$null
  if ($LASTEXITCODE -ne 0) {
    throw "Unable to determine repo dirty state for package provenance."
  }

  if (($output | Out-String).Trim()) {
    return "true"
  }

  "false"
}

function Get-ShortCommit {
  param([string]$Commit)

  if (-not $Commit) {
    return "unknown"
  }

  if ($Commit.Length -le 12) {
    return $Commit
  }

  $Commit.Substring(0, 12)
}

function Get-BinaryBuildMetadata {
  param(
    [string]$BinaryPath,
    [string]$BinaryName
  )

  $metadata = & $BinaryPath version --json | ConvertFrom-Json
  if (-not $metadata.git_commit) {
    throw "$BinaryName build metadata did not report git_commit."
  }

  $metadata
}

function Assert-BinaryCommitMatchesHead {
  param(
    [pscustomobject]$Metadata,
    [string]$BinaryName,
    [string]$RepoHead
  )

  if (-not $Metadata.git_commit.Equals($RepoHead, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "$BinaryName commit $(Get-ShortCommit -Commit $Metadata.git_commit) does not match repo HEAD $(Get-ShortCommit -Commit $RepoHead). Rebuild/package provenance is stale."
  }
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$outputRoot = (Resolve-Path (New-Item -ItemType Directory -Force -Path $OutputDir)).Path
$packageRoot = Join-Path $outputRoot "tablinq"
$binRoot = Join-Path $packageRoot "bin"
$cargoTargetRoot = if ($TargetDir) {
  (Resolve-Path (New-Item -ItemType Directory -Force -Path $TargetDir)).Path
} else {
  Join-Path $repoRoot "target"
}
$repoHead = Get-WorkspaceHeadCommit -RepoRoot $repoRoot
$repoDirty = Get-WorkspaceDirtyState -RepoRoot $repoRoot

if (Test-Path $packageRoot) {
  Remove-Item -Recurse -Force $packageRoot
}

New-Item -ItemType Directory -Force -Path $binRoot | Out-Null
$packageStagingRoot = Join-Path $repoRoot "dist\\package-staging"
$browserAdapterStagingRoot = Join-Path $packageStagingRoot "browser-adapter"
$browserAdapterLegacyStagingRoot = Join-Path $repoRoot "packages\\browser-adapter\\dist\\package-staging"
$browserAdapterPackageRoot = Join-Path $packageRoot "browser-adapter"

Push-Location $repoRoot
try {
  $previousBuildCommit = $env:TABLINQ_BUILD_COMMIT
  $previousBuildDirty = $env:TABLINQ_BUILD_DIRTY
  $env:TABLINQ_BUILD_COMMIT = $repoHead
  $env:TABLINQ_BUILD_DIRTY = $repoDirty
  $cargoArgs = @("build", "--profile", $Profile, "-p", "gpt", "-p", "gptd", "-p", "tablinq-gateway", "-p", "tablinq-host")
  if ($TargetDir) {
    $cargoArgs += @("--target-dir", $cargoTargetRoot)
  }
  Invoke-Step cargo $cargoArgs $repoRoot
  foreach ($distDir in @(
    (Join-Path $repoRoot "packages\\browser-adapter\\dist"),
    (Join-Path $repoRoot "packages\\extension\\dist")
  )) {
    if (Test-Path $distDir) {
      Remove-Item -Recurse -Force $distDir
    }
  }
  Invoke-Step pnpm @("--filter", "@tablinq/browser-adapter", "build") $repoRoot
  Invoke-Step pnpm @("--filter", "@tablinq/extension", "build") $repoRoot
  if (Test-Path $browserAdapterStagingRoot) {
    Remove-Item -Recurse -Force $browserAdapterStagingRoot
  }
  if (Test-Path $browserAdapterLegacyStagingRoot) {
    Remove-Item -Recurse -Force $browserAdapterLegacyStagingRoot
  }
  $browserAdapterDeployTarget = $browserAdapterStagingRoot
  Invoke-Step pnpm @("--filter", "@tablinq/browser-adapter", "deploy", "--legacy", "--prod", $browserAdapterDeployTarget) $repoRoot

  $targetProfileDir = Resolve-TargetProfileDir -Profile $Profile
  $profileTargetDir = Join-Path $cargoTargetRoot $targetProfileDir
  foreach ($binary in @("gpt.exe", "gptd.exe", "tablinq-gateway.exe", "tablinq-host.exe")) {
    Copy-Item -Path (Join-Path $profileTargetDir $binary) -Destination (Join-Path $binRoot $binary) -Force
  }

  Copy-DirectoryContents $browserAdapterStagingRoot $browserAdapterPackageRoot
  Copy-DirectoryContents (Join-Path $repoRoot "packages\\browser-adapter\\dist\\src") (Join-Path $packageRoot "browser-adapter\\src")
  Copy-DirectoryContents (Join-Path $repoRoot "packages\\extension\\dist\\src") (Join-Path $packageRoot "extension\\src")
  Copy-Item -Path (Join-Path $repoRoot "packages\\extension\\manifest.json") -Destination (Join-Path $packageRoot "extension\\manifest.json") -Force
  Copy-DirectoryContents (Join-Path $repoRoot "selector-packs") (Join-Path $packageRoot "selector-packs")
  Copy-DirectoryContents (Join-Path $repoRoot "schemas") (Join-Path $packageRoot "schemas")
  Copy-DirectoryContents (Join-Path $repoRoot "docs") (Join-Path $packageRoot "docs")
  Copy-Item -Path (Join-Path $repoRoot "LICENSE") -Destination (Join-Path $packageRoot "LICENSE") -Force
  Copy-Item -Path (Join-Path $repoRoot "README.md") -Destination (Join-Path $packageRoot "README.md") -Force

  $extensionId = Get-ExtensionIdFromKey -ManifestPath (Join-Path $packageRoot "extension\\manifest.json")
  $buildMetadata = Get-BinaryBuildMetadata -BinaryPath (Join-Path $binRoot "gpt.exe") -BinaryName "gpt.exe"
  Assert-BinaryCommitMatchesHead -Metadata $buildMetadata -BinaryName "gpt.exe" -RepoHead $repoHead
  Assert-BinaryCommitMatchesHead -Metadata (Get-BinaryBuildMetadata -BinaryPath (Join-Path $binRoot "gptd.exe") -BinaryName "gptd.exe") -BinaryName "gptd.exe" -RepoHead $repoHead
  Assert-BinaryCommitMatchesHead -Metadata (Get-BinaryBuildMetadata -BinaryPath (Join-Path $binRoot "tablinq-gateway.exe") -BinaryName "tablinq-gateway.exe") -BinaryName "tablinq-gateway.exe" -RepoHead $repoHead
  $packageMetadata = [ordered]@{
    package_version = $buildMetadata.version
    commit = $buildMetadata.git_commit
    dirty = $buildMetadata.git_dirty
    target = $buildMetadata.target
    profile = $buildMetadata.profile
    built_at_unix_epoch = $buildMetadata.built_at_unix_epoch
    extension_id = $extensionId
    cli_binary = (Join-Path $binRoot "gpt.exe")
    daemon_binary = (Join-Path $binRoot "gptd.exe")
    gateway_binary = (Join-Path $binRoot "tablinq-gateway.exe")
    native_host_binary = (Join-Path $binRoot "tablinq-host.exe")
    extension_path = (Join-Path $packageRoot "extension")
    browser_adapter_entry = (Join-Path $packageRoot "browser-adapter\\src\\service.js")
  }

  $packageMetadata | ConvertTo-Json -Depth 6 | Set-Content -Path (Join-Path $packageRoot "package-metadata.json")
  Write-Host "Packaged Tablinq to $packageRoot"
  Write-Host "Extension ID: $extensionId"
  Write-Host ("Package commit: {0}" -f (Get-ShortCommit -Commit $buildMetadata.git_commit))
}
finally {
  $env:TABLINQ_BUILD_COMMIT = $previousBuildCommit
  $env:TABLINQ_BUILD_DIRTY = $previousBuildDirty
  if (Test-Path $packageStagingRoot) {
    Remove-Item -Recurse -Force $packageStagingRoot
  }
  if (Test-Path $browserAdapterLegacyStagingRoot) {
    Remove-Item -Recurse -Force $browserAdapterLegacyStagingRoot
  }
  Pop-Location
}
