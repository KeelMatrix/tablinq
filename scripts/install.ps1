param(
  [string]$PackageRoot = (Join-Path $PSScriptRoot "..\\dist\\package\\tablinq"),
  [string]$InstallRoot = (Join-Path $env:LOCALAPPDATA "Tablinq\\install"),
  [string]$StateRoot = (Join-Path $env:LOCALAPPDATA "Tablinq"),
  [switch]$AutoStartDaemon,
  [switch]$AutoStartGateway,
  [switch]$SkipRegistry
)

$ErrorActionPreference = "Stop"

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

function Resolve-BrowserExecutable {
  $candidates = @(
    "${env:ProgramFiles(x86)}\\Microsoft\\Edge\\Application\\msedge.exe",
    "$env:ProgramFiles\\Microsoft\\Edge\\Application\\msedge.exe",
    "${env:ProgramFiles(x86)}\\Google\\Chrome\\Application\\chrome.exe",
    "$env:ProgramFiles\\Google\\Chrome\\Application\\chrome.exe"
  ) | Where-Object { $_ }

  foreach ($candidate in $candidates) {
    if (Test-Path $candidate) {
      return $candidate
    }
  }

  foreach ($commandName in @("msedge.exe", "chrome.exe")) {
    $command = Get-Command $commandName -ErrorAction SilentlyContinue
    if ($command) {
      return $command.Source
    }
  }

  return $null
}

function Write-NativeHostManifest {
  param(
    [string]$ManifestPath,
    [string]$HostExecutable,
    [string]$ExtensionId
  )

  $payload = [ordered]@{
    name = "com.tablinq.bridge"
    description = "Tablinq local native messaging host"
    path = $HostExecutable
    type = "stdio"
    allowed_origins = @("chrome-extension://$ExtensionId/")
  }

  $payload | ConvertTo-Json -Depth 4 | Set-Content -Path $ManifestPath
}

function Get-StartupFolder {
  Join-Path $env:APPDATA "Microsoft\\Windows\\Start Menu\\Programs\\Startup"
}

function Write-StartupLauncher {
  param(
    [string]$Path,
    [string]$GptExecutable,
    [string]$StateRoot,
    [string]$CommandText
  )

  $content = @"
@echo off
start "" /min "$GptExecutable" --state-root "$StateRoot" $CommandText
"@
  Set-Content -Path $Path -Value $content -Encoding ascii
}

function Install-StartupLauncher {
  param(
    [string]$SourcePath,
    [string]$FileName
  )

  $startupFolder = Get-StartupFolder
  New-Item -ItemType Directory -Force -Path $startupFolder | Out-Null
  Copy-Item -Path $SourcePath -Destination (Join-Path $startupFolder $FileName) -Force
}

if (-not (Test-Path $PackageRoot)) {
  throw "Package root does not exist: $PackageRoot"
}

if (Test-Path $InstallRoot) {
  Remove-Item -Recurse -Force $InstallRoot
}

New-Item -ItemType Directory -Force -Path $InstallRoot | Out-Null
Copy-Item -Path (Join-Path $PackageRoot "*") -Destination $InstallRoot -Recurse -Force

$binDir = Join-Path $InstallRoot "bin"
$extensionDir = Join-Path $InstallRoot "extension"
$gptBin = Join-Path $binDir "gpt.exe"
$gatewayBin = Join-Path $binDir "tablinq-gateway.exe"
$hostBin = Join-Path $binDir "tablinq-host.exe"
$extensionManifestPath = Join-Path $extensionDir "manifest.json"
$extensionId = Get-ExtensionIdFromKey -ManifestPath $extensionManifestPath
$browserPath = Resolve-BrowserExecutable

$nativeHostDir = Join-Path $InstallRoot "native-host"
New-Item -ItemType Directory -Force -Path $nativeHostDir | Out-Null
$chromeManifestPath = Join-Path $nativeHostDir "com.tablinq.bridge.chrome.json"
$edgeManifestPath = Join-Path $nativeHostDir "com.tablinq.bridge.edge.json"
Write-NativeHostManifest -ManifestPath $chromeManifestPath -HostExecutable $hostBin -ExtensionId $extensionId
Write-NativeHostManifest -ManifestPath $edgeManifestPath -HostExecutable $hostBin -ExtensionId $extensionId

if (-not $SkipRegistry) {
  $chromeRegistryKey = "HKCU:\\Software\\Google\\Chrome\\NativeMessagingHosts\\com.tablinq.bridge"
  $edgeRegistryKey = "HKCU:\\Software\\Microsoft\\Edge\\NativeMessagingHosts\\com.tablinq.bridge"
  New-Item -Path $chromeRegistryKey -Force -Value $chromeManifestPath | Out-Null
  New-Item -Path $edgeRegistryKey -Force -Value $edgeManifestPath | Out-Null
}

& $gptBin --state-root $StateRoot config show | Out-Null

$startupHelperDir = Join-Path $InstallRoot "startup"
New-Item -ItemType Directory -Force -Path $startupHelperDir | Out-Null
$daemonStartupHelper = Join-Path $startupHelperDir "start-tablinq-daemon.cmd"
$gatewayStartupHelper = Join-Path $startupHelperDir "start-tablinq-gateway.cmd"
Write-StartupLauncher -Path $daemonStartupHelper -GptExecutable $gptBin -StateRoot $StateRoot -CommandText "daemon start"
Write-StartupLauncher -Path $gatewayStartupHelper -GptExecutable $gptBin -StateRoot $StateRoot -CommandText "gateway start"

if ($AutoStartDaemon) {
  Install-StartupLauncher -SourcePath $daemonStartupHelper -FileName "Tablinq Daemon.cmd"
}

if ($AutoStartGateway) {
  Install-StartupLauncher -SourcePath $gatewayStartupHelper -FileName "Tablinq Gateway.cmd"
}

Write-Host "Installed Tablinq to $InstallRoot"
Write-Host "State root: $StateRoot"
Write-Host "Detected browser: $(if ($browserPath) { $browserPath } else { 'not found' })"
Write-Host "Extension path: $extensionDir"
Write-Host "Extension ID: $extensionId"
Write-Host "Gateway binary: $gatewayBin"
if ($SkipRegistry) {
  Write-Host "Native host registry registration: skipped"
}
Write-Host "Startup helper scripts:"
Write-Host "  daemon: $daemonStartupHelper"
Write-Host "  gateway: $gatewayStartupHelper"
if ($AutoStartDaemon -or $AutoStartGateway) {
  Write-Host "Startup folder: $(Get-StartupFolder)"
}
Write-Host "First run:"
Write-Host "  1. Load the unpacked extension from $extensionDir in Edge or Chrome."
Write-Host "  2. Run: $gptBin daemon start"
Write-Host "  3. Run: $gptBin gateway start"
Write-Host "  4. Run: $gptBin browser open"
Write-Host "  5. Log in once in the managed browser profile."
Write-Host "  6. Run: $gptBin status"
