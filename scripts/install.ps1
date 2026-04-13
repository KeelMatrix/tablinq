param(
  [string]$PackageRoot = (Join-Path $PSScriptRoot "..\\dist\\package\\tablinq"),
  [string]$InstallRoot = (Join-Path $env:LOCALAPPDATA "Tablinq\\install"),
  [string]$StateRoot = (Join-Path $env:LOCALAPPDATA "Tablinq")
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

if (-not (Test-Path $PackageRoot)) {
  throw "Package root does not exist: $PackageRoot"
}

if (Test-Path $InstallRoot) {
  Remove-Item -Recurse -Force $InstallRoot
}

New-Item -ItemType Directory -Force -Path $InstallRoot | Out-Null
Copy-Item -Path (Join-Path $PackageRoot "*") -Destination $InstallRoot -Recurse -Force

$gptBin = Join-Path $InstallRoot "bin\\gpt.exe"
$hostBin = Join-Path $InstallRoot "bin\\tablinq-host.exe"
$extensionManifestPath = Join-Path $InstallRoot "extension\\manifest.json"
$extensionId = Get-ExtensionIdFromKey -ManifestPath $extensionManifestPath
$browserPath = Resolve-BrowserExecutable

$nativeHostDir = Join-Path $InstallRoot "native-host"
New-Item -ItemType Directory -Force -Path $nativeHostDir | Out-Null
$chromeManifestPath = Join-Path $nativeHostDir "com.tablinq.bridge.chrome.json"
$edgeManifestPath = Join-Path $nativeHostDir "com.tablinq.bridge.edge.json"
Write-NativeHostManifest -ManifestPath $chromeManifestPath -HostExecutable $hostBin -ExtensionId $extensionId
Write-NativeHostManifest -ManifestPath $edgeManifestPath -HostExecutable $hostBin -ExtensionId $extensionId

$chromeRegistryKey = "HKCU:\\Software\\Google\\Chrome\\NativeMessagingHosts\\com.tablinq.bridge"
$edgeRegistryKey = "HKCU:\\Software\\Microsoft\\Edge\\NativeMessagingHosts\\com.tablinq.bridge"
New-Item -Path $chromeRegistryKey -Force -Value $chromeManifestPath | Out-Null
New-Item -Path $edgeRegistryKey -Force -Value $edgeManifestPath | Out-Null

& $gptBin --state-root $StateRoot config show | Out-Null

Write-Host "Installed Tablinq to $InstallRoot"
Write-Host "State root: $StateRoot"
Write-Host "Detected browser: $(if ($browserPath) { $browserPath } else { 'not found' })"
Write-Host "Extension path: $(Join-Path $InstallRoot 'extension')"
Write-Host "Extension ID: $extensionId"
Write-Host "First run:"
Write-Host "  1. Load the unpacked extension from $(Join-Path $InstallRoot 'extension') in Edge or Chrome."
Write-Host "  2. Run: $gptBin daemon start"
Write-Host "  3. Run: $gptBin browser open"
Write-Host "  4. Log in once in the managed browser profile."
