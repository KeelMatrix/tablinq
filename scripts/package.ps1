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

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$outputRoot = (Resolve-Path (New-Item -ItemType Directory -Force -Path $OutputDir)).Path
$packageRoot = Join-Path $outputRoot "tablinq"
$binRoot = Join-Path $packageRoot "bin"
$cargoTargetRoot = if ($TargetDir) {
  (Resolve-Path (New-Item -ItemType Directory -Force -Path $TargetDir)).Path
} else {
  Join-Path $repoRoot "target"
}

if (Test-Path $packageRoot) {
  Remove-Item -Recurse -Force $packageRoot
}

New-Item -ItemType Directory -Force -Path $binRoot | Out-Null
$packageStagingRoot = Join-Path $repoRoot "dist\\package-staging"
$browserAdapterStagingRoot = Join-Path $packageStagingRoot "browser-adapter"
$browserAdapterPackageRoot = Join-Path $packageRoot "browser-adapter"

Push-Location $repoRoot
try {
  $cargoArgs = @("build", "--profile", $Profile, "-p", "gpt", "-p", "gptd", "-p", "tablinq-gateway", "-p", "tablinq-host")
  if ($TargetDir) {
    $cargoArgs += @("--target-dir", $cargoTargetRoot)
  }
  Invoke-Step cargo $cargoArgs $repoRoot
  Invoke-Step pnpm @("build") $repoRoot
  if (Test-Path $browserAdapterStagingRoot) {
    Remove-Item -Recurse -Force $browserAdapterStagingRoot
  }
  New-Item -ItemType Directory -Force -Path $browserAdapterStagingRoot | Out-Null
  $browserAdapterDeployTarget = Get-RelativePath $repoRoot $browserAdapterStagingRoot
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
  $buildMetadata = & (Join-Path $binRoot "gpt.exe") version --json | ConvertFrom-Json
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
}
finally {
  if (Test-Path $packageStagingRoot) {
    Remove-Item -Recurse -Force $packageStagingRoot
  }
  Pop-Location
}
