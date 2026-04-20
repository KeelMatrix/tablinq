param(
  [string]$Profile = "release",
  [string]$PackageRoot = (Join-Path $PSScriptRoot "..\dist\package\tablinq"),
  [string]$InstallRoot = (Join-Path $env:LOCALAPPDATA "Tablinq\install"),
  [string]$StateRoot = (Join-Path $env:LOCALAPPDATA "Tablinq"),
  [switch]$SkipRegistry,
  [switch]$EnableAutoStart,
  [switch]$LaunchAfterInstall,
  [switch]$SkipPathUpdate,
  [switch]$SkipDependencyInstall,
  [switch]$SkipPackageBuild
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$packageScriptPath = Join-Path $PSScriptRoot "package.ps1"
$daemonTaskName = "Tablinq Daemon"
$gatewayTaskName = "Tablinq Gateway"
$legacyDaemonLaunchers = @("Tablinq Daemon.cmd")
$legacyGatewayLaunchers = @("Tablinq Gateway.cmd")

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

function Assert-CommandAvailable {
  param(
    [string]$CommandName,
    [string]$InstallHint
  )

  if (-not (Get-Command $CommandName -ErrorAction SilentlyContinue)) {
    throw "Missing required command '$CommandName'. $InstallHint"
  }
}

function Remove-PathIfExists {
  param([string]$Path)

  if (Test-Path $Path) {
    Remove-Item -Recurse -Force $Path
  }
}

function Assert-PathExists {
  param(
    [string]$Path,
    [string]$Label
  )

  if (-not (Test-Path $Path)) {
    throw "$Label is missing at $Path. This is commonly caused by antivirus quarantine or a blocked file operation. Add an exception for the repo root and %LOCALAPPDATA%\Tablinq, restore the blocked file, and rerun scripts/install.ps1."
  }
}

function Assert-DirectoryWritable {
  param(
    [string]$Path,
    [string]$Label
  )

  if (-not $Path -or -not $Path.Trim()) {
    throw "Missing required path for $Label."
  }

  New-Item -ItemType Directory -Force -Path $Path | Out-Null
  $probePath = Join-Path $Path (".tablinq-write-test-{0}.tmp" -f ([guid]::NewGuid().ToString("N")))
  try {
    Set-Content -Path $probePath -Value "ok" -Encoding ascii -ErrorAction Stop
  }
  catch {
    throw "$Label is not writable at $Path. Grant write access or choose a different install location."
  }
  finally {
    Remove-Item -Path $probePath -Force -ErrorAction SilentlyContinue
  }
}

function Assert-RegistryWritable {
  param([string]$KeyPath)

  Assert-CommandAvailable -CommandName "reg.exe" -InstallHint "Windows registry tooling is required."
  $probeName = "tablinq-install-prereq-{0}" -f ([guid]::NewGuid().ToString("N"))
  $fullKeyPath = "$KeyPath\$probeName"
  $stdoutPath = [System.IO.Path]::GetTempFileName()
  $stderrPath = [System.IO.Path]::GetTempFileName()
  try {
    $add = Start-Process -FilePath "reg.exe" -ArgumentList @("add", $fullKeyPath, "/f") -Wait -PassThru -NoNewWindow -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    if ($add.ExitCode -ne 0) {
      throw "HKCU registry is not writable at $KeyPath. Run with -SkipRegistry or use an account that can write per-user registry keys."
    }
  }
  finally {
    Start-Process -FilePath "reg.exe" -ArgumentList @("delete", $fullKeyPath, "/f") -Wait -NoNewWindow -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath | Out-Null
    Remove-Item -Force $stdoutPath, $stderrPath -ErrorAction SilentlyContinue
  }
}

function Get-ExtensionIdFromKey {
  param([string]$ManifestPath)

  $manifest = Get-Content $ManifestPath | ConvertFrom-Json
  if (-not $manifest.key) {
    throw "Missing extension key: $ManifestPath"
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

function Resolve-BrowserSelection {
  $candidates = @(
    [pscustomobject]@{ Name = "Chrome"; ConfigChannel = "chrome"; Path = Join-Path $env:LOCALAPPDATA "Google\Chrome\Application\chrome.exe" }
    [pscustomobject]@{ Name = "Chrome"; ConfigChannel = "chrome"; Path = "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe" }
    [pscustomobject]@{ Name = "Chrome"; ConfigChannel = "chrome"; Path = "$env:ProgramFiles\Google\Chrome\Application\chrome.exe" }
    [pscustomobject]@{ Name = "Edge"; ConfigChannel = "msedge"; Path = Join-Path $env:LOCALAPPDATA "Microsoft\Edge\Application\msedge.exe" }
    [pscustomobject]@{ Name = "Edge"; ConfigChannel = "msedge"; Path = "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe" }
    [pscustomobject]@{ Name = "Edge"; ConfigChannel = "msedge"; Path = "$env:ProgramFiles\Microsoft\Edge\Application\msedge.exe" }
  ) | Where-Object { $_.Path -and $_.Path.Trim() }

  foreach ($candidate in $candidates) {
    if (Test-Path $candidate.Path) {
      return $candidate
    }
  }

  foreach ($commandName in @("chrome.exe", "msedge.exe")) {
    $resolved = (& where.exe $commandName 2>$null | Select-Object -First 1)
    if ($LASTEXITCODE -eq 0 -and $resolved) {
      return [pscustomobject]@{
        Name = if ($commandName -eq "chrome.exe") { "Chrome" } else { "Edge" }
        ConfigChannel = if ($commandName -eq "chrome.exe") { "chrome" } else { "msedge" }
        Path = $resolved.Trim()
      }
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

  $payload | ConvertTo-Json -Depth 4 | Set-Content -Path $ManifestPath -Encoding ascii
}

function Ensure-StateLayout {
  param([string]$StateRoot)

  foreach ($dir in @(
    $StateRoot,
    (Join-Path $StateRoot "config"),
    (Join-Path $StateRoot "logs"),
    (Join-Path $StateRoot "db"),
    (Join-Path $StateRoot "browser-profile"),
    (Join-Path $StateRoot "chrome-attach-profile"),
    (Join-Path $StateRoot "downloads"),
    (Join-Path $StateRoot "crash-bundles"),
    (Join-Path $StateRoot "gateway-uploads")
  )) {
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
  }
}

function Normalize-PageAdapterRuntime {
  param([string]$RuntimePath)

  if (-not (Test-Path $RuntimePath)) {
    return
  }

  $lines = Get-Content -Path $RuntimePath
  $normalized = foreach ($line in $lines) {
    if (
      $line.Trim() -eq '"use strict";' -or
      ($line.Contains('Object.defineProperty(exports') -and $line.Contains('"__esModule"')) -or
      $line.Contains('exports.createTablinqPageAdapterRuntime =')
    ) {
      continue
    }

    $line -replace '^\s*export\s+function\s+createTablinqPageAdapterRuntime', 'function createTablinqPageAdapterRuntime'
  }

  Set-Content -Path $RuntimePath -Value $normalized -Encoding utf8
}

function Convert-ToTomlStringLiteral {
  param([string]$Value)

  "'" + ($Value.Replace("'", "''")) + "'"
}

function Set-TomlStringValue {
  param(
    [string]$Path,
    [string]$Key,
    [string]$Value
  )

  $line = "{0} = {1}" -f $Key, (Convert-ToTomlStringLiteral -Value $Value)
  $content = if (Test-Path $Path) { Get-Content $Path -Raw } else { "" }
  $keyPattern = "(?m)^{0}\s*=.*$" -f [regex]::Escape($Key)

  if ($content -match $keyPattern) {
    $content = [regex]::Replace($content, $keyPattern, [System.Text.RegularExpressions.MatchEvaluator]{ param($match) $line }, 1)
  }
  elseif ($content -match "(?m)^\[gateway\]\s*$") {
    $content = [regex]::Replace(
      $content,
      "(?m)^\[gateway\]\s*$",
      [System.Text.RegularExpressions.MatchEvaluator]{ param($match) $line + "`r`n`r`n[gateway]" },
      1
    )
  }
  else {
    if ($content -and -not $content.EndsWith("`n")) {
      $content += "`r`n"
    }
    $content += $line + "`r`n"
  }

  Set-Content -Path $Path -Value $content -Encoding utf8
}

function Get-TomlStringValue {
  param(
    [string]$Path,
    [string]$Key,
    [string]$DefaultValue = ""
  )

  if (-not (Test-Path $Path)) {
    return $DefaultValue
  }

  $content = Get-Content -Path $Path -Raw
  $pattern = '(?m)^{0}\s*=\s*([''"])(?<value>.*?)\1\s*$' -f [regex]::Escape($Key)
  $match = [regex]::Match($content, $pattern)
  if ($match.Success) {
    return $match.Groups["value"].Value
  }

  return $DefaultValue
}

function Ensure-ConfigFile {
  param([string]$StateRoot)

  $configPath = Join-Path $StateRoot "config\config.toml"
  if (-not (Test-Path $configPath)) {
    @(
      "version = 2"
      "warm_mode = false"
      "safe_mode = false"
      "browser_target_url = 'https://chatgpt.com/'"
      ""
      "[gateway]"
      "host = '127.0.0.1'"
      "port = 3847"
    ) | Set-Content -Path $configPath -Encoding utf8
  }

  $configPath
}

function Ensure-ManagedBrowserState {
  param([string]$StateRoot)

  $profileDir = Join-Path $StateRoot "browser-profile"
  New-Item -ItemType Directory -Force -Path $profileDir | Out-Null
}

function Stop-ManagedProcessByPath {
  param([string]$ExecutablePath)

  if (-not (Test-Path $ExecutablePath)) {
    return
  }

  $normalized = [System.IO.Path]::GetFullPath($ExecutablePath)
  Get-Process | Where-Object {
    $_.Path -and $_.Path.Equals($normalized, [System.StringComparison]::OrdinalIgnoreCase)
  } | ForEach-Object {
    Stop-Process -Id $_.Id -Force -ErrorAction SilentlyContinue
  }
}

function Stop-ManagedBrowserProfileProcesses {
  param([string]$StateRoot)

  $profileDir = Join-Path $StateRoot "browser-profile"
  $normalizedProfileDir = [System.IO.Path]::GetFullPath($profileDir)
  $normalizedStateRoot = [System.IO.Path]::GetFullPath($StateRoot)
  $processes = Get-CimInstance Win32_Process -Filter "Name = 'chrome.exe' OR Name = 'msedge.exe'" -ErrorAction SilentlyContinue
  foreach ($process in $processes) {
    $commandLine = $process.CommandLine
    if (-not $commandLine) {
      continue
    }

    if (
      $commandLine.IndexOf($normalizedProfileDir, [System.StringComparison]::OrdinalIgnoreCase) -ge 0 -or
      $commandLine.IndexOf($normalizedStateRoot, [System.StringComparison]::OrdinalIgnoreCase) -ge 0
    ) {
      Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue
    }
  }
}

function Remove-LegacyStartupLaunchers {
  $startupFolder = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Startup"
  foreach ($fileName in @($legacyDaemonLaunchers + $legacyGatewayLaunchers)) {
    $path = Join-Path $startupFolder $fileName
    if (Test-Path $path) {
      try {
        Remove-Item -Force $path -ErrorAction Stop
      }
      catch {
        Write-Warning "Could not remove legacy startup launcher: $path"
      }
    }
  }
}

function Remove-AutoStartScheduledTask {
  param([string]$TaskName)

  $stdoutPath = [System.IO.Path]::GetTempFileName()
  $stderrPath = [System.IO.Path]::GetTempFileName()
  try {
    $query = Start-Process -FilePath "schtasks.exe" -ArgumentList @("/Query", "/TN", $TaskName) -Wait -PassThru -NoNewWindow -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    if ($query.ExitCode -eq 0) {
      Start-Process -FilePath "schtasks.exe" -ArgumentList @("/Delete", "/TN", $TaskName, "/F") -Wait -NoNewWindow -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath | Out-Null
    }
  }
  finally {
    Remove-Item -Force $stdoutPath, $stderrPath -ErrorAction SilentlyContinue
  }
}

function Remove-RunKeyValue {
  param([string]$ValueName)

  $runKeyPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
  if (-not (Test-Path $runKeyPath)) {
    return
  }

  try {
    $value = Get-ItemProperty -Path $runKeyPath -Name $ValueName -ErrorAction Stop
    if ($null -ne $value) {
      Remove-ItemProperty -Path $runKeyPath -Name $ValueName -ErrorAction Stop
    }
  }
  catch [System.Management.Automation.PSArgumentException] {
  }
  catch [System.Management.Automation.ItemNotFoundException] {
  }
}

function Remove-AutoStartPersistence {
  Remove-LegacyStartupLaunchers
  foreach ($taskName in @($daemonTaskName, $gatewayTaskName)) {
    Remove-AutoStartScheduledTask -TaskName $taskName
    Remove-RunKeyValue -ValueName $taskName
  }

  $helperDir = Join-Path $InstallRoot "startup"
  if (Test-Path $helperDir) {
    Remove-Item -Recurse -Force $helperDir -ErrorAction SilentlyContinue
  }
}

function Try-RegisterRegistryDefaultValue {
  param(
    [string]$KeyPath,
    [string]$Value
  )

  $stdoutPath = [System.IO.Path]::GetTempFileName()
  $stderrPath = [System.IO.Path]::GetTempFileName()
  try {
    $process = Start-Process -FilePath "reg.exe" -ArgumentList @("add", $KeyPath, "/ve", "/t", "REG_SZ", "/d", $Value, "/f") -Wait -PassThru -NoNewWindow -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    $process.ExitCode -eq 0
  }
  finally {
    Remove-Item -Force $stdoutPath, $stderrPath -ErrorAction SilentlyContinue
  }
}

function New-AutoStartScheduledTask {
  param(
    [string]$TaskName,
    [string]$ExecutablePath,
    [string]$StateRoot
  )

  $taskCommand = ('"{0}" serve --state-root "{1}"' -f $ExecutablePath, $StateRoot)
  $stdoutPath = [System.IO.Path]::GetTempFileName()
  $stderrPath = [System.IO.Path]::GetTempFileName()
  try {
    $process = Start-Process -FilePath "schtasks.exe" -ArgumentList @(
      "/Create",
      "/SC",
      "ONLOGON",
      "/TN",
      $TaskName,
      "/TR",
      $taskCommand,
      "/F"
    ) -Wait -PassThru -NoNewWindow -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    if ($process.ExitCode -ne 0) {
      throw "Could not create autostart scheduled task '$TaskName'."
    }
  }
  finally {
    Remove-Item -Force $stdoutPath, $stderrPath -ErrorAction SilentlyContinue
  }
}

function Install-AutoStartPersistence {
  param(
    [string]$DaemonExecutable,
    [string]$GatewayExecutable,
    [string]$StateRoot,
    [string]$InstallRoot
  )

  Remove-AutoStartPersistence
  New-AutoStartScheduledTask -TaskName $daemonTaskName -ExecutablePath $DaemonExecutable -StateRoot $StateRoot
  New-AutoStartScheduledTask -TaskName $gatewayTaskName -ExecutablePath $GatewayExecutable -StateRoot $StateRoot
  return "scheduled_tasks"
}

function Add-UserPathEntry {
  param([string]$Entry)

  $normalizedEntry = $Entry.Trim().TrimEnd("\")
  if (-not $normalizedEntry) {
    return
  }

  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  $userParts = @()
  if ($userPath) {
    $userParts = $userPath.Split(";") | Where-Object { $_.Trim() }
  }

  $containsUserEntry = $userParts | Where-Object {
    $_.Trim().TrimEnd("\").Equals($normalizedEntry, [System.StringComparison]::OrdinalIgnoreCase)
  }

  if (-not $containsUserEntry) {
    $updatedUserPath = if ($userPath -and $userPath.Trim()) { $userPath.TrimEnd(";") + ";" + $Entry } else { $Entry }
    [Environment]::SetEnvironmentVariable("Path", $updatedUserPath, "User")
  }

  $processParts = $env:Path.Split(";") | Where-Object { $_.Trim() }
  $containsProcessEntry = $processParts | Where-Object {
    $_.Trim().TrimEnd("\").Equals($normalizedEntry, [System.StringComparison]::OrdinalIgnoreCase)
  }
  if (-not $containsProcessEntry) {
    $env:Path = if ($env:Path -and $env:Path.Trim()) { $env:Path.TrimEnd(";") + ";" + $Entry } else { $Entry }
  }
}

function Start-ManagedBinary {
  param(
    [string]$ExecutablePath,
    [string]$StateRoot
  )

  Start-Process -FilePath $ExecutablePath -ArgumentList @("serve", "--state-root", $StateRoot) -WindowStyle Hidden | Out-Null
}

function Test-ManagedBinaryRunning {
  param([string]$ExecutablePath)

  if (-not (Test-Path $ExecutablePath)) {
    return $false
  }

  $normalized = [System.IO.Path]::GetFullPath($ExecutablePath)
  return [bool](Get-Process | Where-Object {
    $_.Path -and $_.Path.Equals($normalized, [System.StringComparison]::OrdinalIgnoreCase)
  } | Select-Object -First 1)
}

function Test-DaemonResponsive {
  param(
    [string]$CliExecutablePath,
    [string]$StateRoot
  )

  if (-not (Test-Path $CliExecutablePath)) {
    return $false
  }

  & $CliExecutablePath --state-root $StateRoot daemon status | Out-Null
  return ($LASTEXITCODE -eq 0)
}

function Write-GptWrapper {
  param(
    [string]$WrapperPath,
    [string]$CliExecutable
  )

  @(
    "@echo off"
    "`"%~dp0tablinq-cli.exe`" %*"
  ) | Set-Content -Path $WrapperPath -Encoding ascii
}

function Invoke-InternalCli {
  param(
    [string]$ExecutablePath,
    [string]$StateRoot,
    [string[]]$Arguments
  )

  & $ExecutablePath --state-root $StateRoot @Arguments | Out-Null
  if ($LASTEXITCODE -ne 0) {
    throw "Command failed: $($Arguments -join ' ')"
  }
}

function Write-ManualSteps {
  param(
    [string]$InstallRoot,
    [string]$StateRoot,
    [string]$BrowserName,
    [string]$ConnectionMode,
    [string]$TargetUrl,
    [bool]$DaemonRunning,
    [bool]$GatewayRunning
  )

  $extensionPath = Join-Path $InstallRoot "extension"
  $gptPath = Join-Path $InstallRoot "bin\gpt.exe"
  $chromeAttachProfilePath = Join-Path $StateRoot "chrome-attach-profile"
  $steps = [System.Collections.Generic.List[string]]::new()

  if (-not $DaemonRunning) {
    $steps.Add("Start the daemon: $gptPath daemon start") | Out-Null
  }

  if (-not $GatewayRunning) {
    $steps.Add("Start the gateway: $gptPath gateway start") | Out-Null
  }

  switch ($ConnectionMode) {
    "managed_only" {
      $steps.Add("Run: $gptPath browser open") | Out-Null
      $steps.Add("Sign in manually at $TargetUrl in the managed browser window if prompted.") | Out-Null
    }
    "managed_fallback" {
      $steps.Add("Run: $gptPath browser open") | Out-Null
      $steps.Add("Sign in manually at $TargetUrl in the managed browser window if prompted.") | Out-Null
    }
    default {
      switch ($BrowserName) {
        "Chrome" {
          $steps.Add("Fully close Chrome.") | Out-Null
          $steps.Add("Run: $gptPath browser start-debug") | Out-Null
          $steps.Add("In the Chrome window opened by that command, open chrome://extensions/, enable Developer mode, remove any previously loaded Tablinq extension in that attach profile, and load the unpacked extension from: $extensionPath") | Out-Null
          $steps.Add("Sign in manually at $TargetUrl in that same dedicated Chrome attach profile.") | Out-Null
        }
        default {
          $steps.Add("Open edge://extensions/ in the Edge profile you will attach to, enable Developer mode, remove any previously loaded Tablinq extension, and load the unpacked extension from: $extensionPath") | Out-Null
          $steps.Add("Fully close Edge.") | Out-Null
          $steps.Add("Run: $gptPath browser start-debug") | Out-Null
          $steps.Add("Sign in manually at $TargetUrl in that same Edge profile.") | Out-Null
        }
      }

      $steps.Add("Run: $gptPath browser attach") | Out-Null
    }
  }

  $steps.Add("Verify browser session: $gptPath browser connection-info") | Out-Null
  $steps.Add("Verify overall status: $gptPath status") | Out-Null
  $steps.Add("Verify diagnostics: $gptPath doctor") | Out-Null
  $steps.Add("Inspect capabilities: $gptPath capabilities") | Out-Null
  $steps.Add("Try a smoke prompt: $gptPath ask `"hello`"") | Out-Null

  Write-Host ""
  Write-Host "Tablinq install is ready."
  Write-Host "Install root: $InstallRoot"
  Write-Host "State root: $StateRoot"
  Write-Host "Browser: $BrowserName"
  Write-Host "Connection mode: $ConnectionMode"
  Write-Host "Daemon running: $DaemonRunning"
  Write-Host "Gateway running: $GatewayRunning"

  if ($ConnectionMode -eq "attach_first" -and $BrowserName -eq "Chrome") {
    Write-Host "Chrome attach profile: $chromeAttachProfilePath"
  }

  Write-Host ""
  Write-Host "Manual steps:"

  for ($index = 0; $index -lt $steps.Count; $index += 1) {
    Write-Host (("{0}. {1}" -f ($index + 1), $steps[$index]))
  }

  if ($ConnectionMode -eq "managed_only" -or $ConnectionMode -eq "managed_fallback") {
    Write-Host ""
    Write-Host "Managed browser mode is a fallback path. Use attach-first unless your machine or browser profile cannot expose the local debug endpoint."
  }
}

Write-Host "Preparing Tablinq Windows install from repo checkout..."
Assert-CommandAvailable -CommandName "cargo" -InstallHint "Install Rust and make sure cargo is on PATH."
Assert-CommandAvailable -CommandName "node" -InstallHint "Install Node.js 20 or newer and make sure node is on PATH."
Assert-CommandAvailable -CommandName "pnpm" -InstallHint "Install pnpm and make sure it is on PATH."
Assert-CommandAvailable -CommandName "powershell" -InstallHint "Windows PowerShell is required and must be on PATH."

$browserSelection = Resolve-BrowserSelection
if (-not $browserSelection) {
  throw "Missing required browser. Install Google Chrome or Microsoft Edge and make sure the browser executable is discoverable on this PC."
}

$localAppDataRoot = [Environment]::GetFolderPath("LocalApplicationData")
if (-not $localAppDataRoot) {
  throw "LOCALAPPDATA is unavailable for the current user. A per-user install location is required."
}

$appDataRoot = [Environment]::GetFolderPath("ApplicationData")
if (-not $appDataRoot) {
  throw "APPDATA is unavailable for the current user. A per-user profile is required."
}

Assert-DirectoryWritable -Path $localAppDataRoot -Label "Local application data folder"
Assert-DirectoryWritable -Path $InstallRoot -Label "Install root"
Assert-DirectoryWritable -Path $StateRoot -Label "State root"

if (-not $SkipRegistry) {
  Assert-RegistryWritable -KeyPath "HKCU\Software"
}

if (-not $SkipDependencyInstall) {
  Write-Host "Installing workspace dependencies with pnpm..."
  Invoke-Step -FilePath "pnpm" -ArgumentList @("install") -WorkingDirectory $repoRoot
}

if (-not $SkipPackageBuild) {
  Write-Host "Building and packaging Tablinq..."
  Invoke-Step -FilePath "powershell" -ArgumentList @(
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    $packageScriptPath,
    "-Profile",
    $Profile
  ) -WorkingDirectory $repoRoot
}

if (-not (Test-Path $PackageRoot)) {
  throw "Missing package root after packaging: $PackageRoot"
}

$existingBinDir = Join-Path $InstallRoot "bin"
Stop-ManagedProcessByPath -ExecutablePath (Join-Path $existingBinDir "tablinq-gateway.exe")
Stop-ManagedProcessByPath -ExecutablePath (Join-Path $existingBinDir "gptd.exe")
Stop-ManagedBrowserProfileProcesses -StateRoot $StateRoot
Start-Sleep -Milliseconds 750

$stagingRoot = "$InstallRoot.new"
$backupRoot = "$InstallRoot.old"
Remove-PathIfExists -Path $stagingRoot
Remove-PathIfExists -Path $backupRoot

New-Item -ItemType Directory -Force -Path $stagingRoot | Out-Null
Copy-Item -Path (Join-Path $PackageRoot "*") -Destination $stagingRoot -Recurse -Force -Exclude "bin"

$binDir = Join-Path $stagingRoot "bin"
$publicCliBin = Join-Path $binDir "gpt.exe"
$internalCliBin = Join-Path $binDir "tablinq-cli.exe"
$daemonBin = Join-Path $binDir "gptd.exe"
$gatewayBin = Join-Path $binDir "tablinq-gateway.exe"
$hostBin = Join-Path $InstallRoot "bin\tablinq-host.exe"
$gptWrapper = Join-Path $binDir "gpt.cmd"
$extensionDir = Join-Path $stagingRoot "extension"
$extensionManifestPath = Join-Path $extensionDir "manifest.json"
$extensionId = Get-ExtensionIdFromKey -ManifestPath $extensionManifestPath
$pageAdapterRuntimePath = Join-Path $stagingRoot "browser-adapter\src\pageAdapter.runtime.js"

New-Item -ItemType Directory -Force -Path $binDir | Out-Null
Copy-Item -Path (Join-Path $PackageRoot "bin\gpt.exe") -Destination $publicCliBin -Force
Copy-Item -Path (Join-Path $PackageRoot "bin\gpt.exe") -Destination $internalCliBin -Force
Copy-Item -Path (Join-Path $PackageRoot "bin\gptd.exe") -Destination $daemonBin -Force
Copy-Item -Path (Join-Path $PackageRoot "bin\tablinq-gateway.exe") -Destination $gatewayBin -Force
Copy-Item -Path (Join-Path $PackageRoot "bin\tablinq-host.exe") -Destination (Join-Path $binDir "tablinq-host.exe") -Force
Write-GptWrapper -WrapperPath $gptWrapper -CliExecutable $internalCliBin
Normalize-PageAdapterRuntime -RuntimePath $pageAdapterRuntimePath

Ensure-StateLayout -StateRoot $StateRoot
$configPath = Ensure-ConfigFile -StateRoot $StateRoot

$nativeHostDir = Join-Path $stagingRoot "native-host"
if (Test-Path $nativeHostDir) {
  Remove-Item -Recurse -Force $nativeHostDir
}
New-Item -ItemType Directory -Force -Path $nativeHostDir | Out-Null

$chromeManifestPath = Join-Path $nativeHostDir "com.tablinq.bridge.chrome.json"
$edgeManifestPath = Join-Path $nativeHostDir "com.tablinq.bridge.edge.json"
Write-NativeHostManifest -ManifestPath $chromeManifestPath -HostExecutable $hostBin -ExtensionId $extensionId
Write-NativeHostManifest -ManifestPath $edgeManifestPath -HostExecutable $hostBin -ExtensionId $extensionId

if (-not $SkipRegistry) {
  foreach ($registryKey in @(
    "HKCU:\Software\Google\Chrome\NativeMessagingHosts\com.tablinq.bridge",
    "HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\com.tablinq.bridge"
  )) {
    if (Test-Path $registryKey) {
      Remove-Item -Path $registryKey -Recurse -Force
    }
  }

  [void](Try-RegisterRegistryDefaultValue -KeyPath "HKCU\Software\Google\Chrome\NativeMessagingHosts\com.tablinq.bridge" -Value $chromeManifestPath)
  [void](Try-RegisterRegistryDefaultValue -KeyPath "HKCU\Software\Microsoft\Edge\NativeMessagingHosts\com.tablinq.bridge" -Value $edgeManifestPath)
}

Set-TomlStringValue -Path $configPath -Key "browser_path" -Value $browserSelection.Path
Set-TomlStringValue -Path $configPath -Key "browser_channel" -Value $browserSelection.ConfigChannel
Ensure-ManagedBrowserState -StateRoot $StateRoot

$connectionMode = Get-TomlStringValue -Path $configPath -Key "browser_connection_mode" -DefaultValue "attach_first"
$targetUrl = Get-TomlStringValue -Path $configPath -Key "browser_target_url" -DefaultValue "https://chatgpt.com/"

if (-not $SkipPathUpdate) {
  Add-UserPathEntry -Entry (Join-Path $InstallRoot "bin")
}

if (Test-Path $InstallRoot) {
  Rename-Item -Path $InstallRoot -NewName ([System.IO.Path]::GetFileName($backupRoot)) -Force
}
Rename-Item -Path $stagingRoot -NewName ([System.IO.Path]::GetFileName($InstallRoot)) -Force
Remove-PathIfExists -Path $backupRoot

Assert-PathExists -Path (Join-Path $InstallRoot "bin\gpt.exe") -Label "Installed CLI"
Assert-PathExists -Path (Join-Path $InstallRoot "bin\gptd.exe") -Label "Installed daemon"
Assert-PathExists -Path (Join-Path $InstallRoot "bin\tablinq-gateway.exe") -Label "Installed gateway"
Assert-PathExists -Path (Join-Path $InstallRoot "bin\tablinq-host.exe") -Label "Installed native host"
Assert-PathExists -Path (Join-Path $InstallRoot "extension\manifest.json") -Label "Installed extension"

Remove-AutoStartPersistence

$autoStartMode = "disabled"
if ($EnableAutoStart) {
  $autoStartMode = Install-AutoStartPersistence -DaemonExecutable (Join-Path $InstallRoot "bin\gptd.exe") -GatewayExecutable (Join-Path $InstallRoot "bin\tablinq-gateway.exe") -StateRoot $StateRoot -InstallRoot $InstallRoot
}

if ($LaunchAfterInstall) {
  Start-ManagedBinary -ExecutablePath (Join-Path $InstallRoot "bin\gptd.exe") -StateRoot $StateRoot
  Start-Sleep -Seconds 2
  Assert-PathExists -Path (Join-Path $InstallRoot "bin\gptd.exe") -Label "Installed daemon"
  Start-ManagedBinary -ExecutablePath (Join-Path $InstallRoot "bin\tablinq-gateway.exe") -StateRoot $StateRoot
  Start-Sleep -Seconds 2
  Assert-PathExists -Path (Join-Path $InstallRoot "bin\tablinq-gateway.exe") -Label "Installed gateway"
}

$daemonRunning = Test-DaemonResponsive -CliExecutablePath (Join-Path $InstallRoot "bin\gpt.exe") -StateRoot $StateRoot
$gatewayRunning = Test-ManagedBinaryRunning -ExecutablePath (Join-Path $InstallRoot "bin\tablinq-gateway.exe")

Write-Host "Auto-start: enabled ($autoStartMode)"
Write-Host "Browser: $($browserSelection.Name)"
Write-ManualSteps -InstallRoot $InstallRoot -StateRoot $StateRoot -BrowserName $browserSelection.Name -ConnectionMode $connectionMode -TargetUrl $targetUrl -DaemonRunning $daemonRunning -GatewayRunning $gatewayRunning
