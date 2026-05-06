param(
  [string]$Profile = "release",
  [string]$PackageRoot = (Join-Path $PSScriptRoot "..\dist\package\tablinq"),
  [string]$InstallRoot = (Join-Path $env:LOCALAPPDATA "Tablinq\install"),
  [string]$StateRoot = (Join-Path $env:LOCALAPPDATA "Tablinq"),
  [switch]$SkipRegistry,
  [switch]$EnableAutoStart,
  [switch]$NoLaunchAfterInstall,
  [switch]$SkipPathUpdate,
  [switch]$SkipDependencyInstall,
  [switch]$SkipPackageBuild
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$packageScriptPath = Join-Path $PSScriptRoot "package.ps1"
$explicitPackageRoot = $PSBoundParameters.ContainsKey("PackageRoot")
if ($explicitPackageRoot -and (Test-Path $PackageRoot)) {
  $PackageRoot = (Resolve-Path $PackageRoot).Path
}
$usePrebuiltPackage = $explicitPackageRoot -and (Test-Path $PackageRoot)
$effectiveSkipDependencyInstall = $SkipDependencyInstall -or $usePrebuiltPackage
$effectiveSkipPackageBuild = $SkipPackageBuild -or $usePrebuiltPackage
$daemonTaskName = "Tablinq Daemon"
$gatewayTaskName = "Tablinq Gateway"
$script:InstallProgressId = 8701
$script:CurrentPhase = 0
$script:TotalPhases = 4
if (-not $effectiveSkipDependencyInstall) {
  $script:TotalPhases += 1
}
if (-not $effectiveSkipPackageBuild) {
  $script:TotalPhases += 1
}
if (-not $NoLaunchAfterInstall) {
  $script:TotalPhases += 1
}

#region Progress and command helpers

function Update-InstallProgress {
  param([string]$Status)

  if ($Host.Name -eq "ServerRemoteHost") {
    return
  }

  $percent = 0
  if ($script:TotalPhases -gt 0) {
    $percent = [math]::Min([math]::Round(($script:CurrentPhase / $script:TotalPhases) * 100), 100)
  }

  Write-Progress -Id $script:InstallProgressId -Activity "Installing Tablinq" -Status $Status -PercentComplete $percent
}

function Complete-InstallProgress {
  Write-Progress -Id $script:InstallProgressId -Activity "Installing Tablinq" -Completed
}

function Start-InstallPhase {
  param([string]$Title)

  $script:CurrentPhase += 1
  Write-Host ""
  Write-Host ("[{0}/{1}] {2}" -f $script:CurrentPhase, $script:TotalPhases, $Title)
  Update-InstallProgress -Status $Title
}

function Write-StatusLine {
  param(
    [string]$Label,
    [string]$State,
    [string]$Details = ""
  )

  if ($Details) {
    Write-Host ("  {0,-12} {1,-12} {2}" -f ($Label + ":"), $State, $Details)
  }
  else {
    Write-Host ("  {0,-12} {1}" -f ($Label + ":"), $State)
  }
}

function Show-CommandFailureLog {
  param(
    [string]$StdoutPath,
    [string]$StderrPath
  )

  foreach ($item in @(
    @{ Label = "stdout"; Path = $StdoutPath },
    @{ Label = "stderr"; Path = $StderrPath }
  )) {
    if (-not (Test-Path $item.Path)) {
      continue
    }

    $content = Get-Content -Path $item.Path -Tail 40 -ErrorAction SilentlyContinue
    if (-not $content) {
      continue
    }

    Write-Host ("  {0} tail:" -f $item.Label)
    foreach ($line in $content) {
      Write-Host ("    {0}" -f $line)
    }
  }
}

function Convert-ToCmdArgument {
  param([string]$Value)

  if ($null -eq $Value -or $Value -eq "") {
    return '""'
  }

  $escaped = $Value.Replace('"', '\"')
  if ($escaped -match '[\s"&^|<>()]') {
    return '"' + $escaped + '"'
  }

  return $escaped
}

function Format-ArgumentList {
  param([string[]]$ArgumentList = @())

  if (-not $ArgumentList -or $ArgumentList.Count -eq 0) {
    return ""
  }

  return (($ArgumentList | ForEach-Object { Convert-ToCmdArgument -Value $_ }) -join " ")
}

function Resolve-ExecutablePath {
  param([string]$FilePath)

  if ([System.IO.Path]::IsPathRooted($FilePath)) {
    return $FilePath
  }

  $commandInfo = Get-Command $FilePath -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $commandInfo) {
    return $FilePath
  }

  foreach ($property in @("Source", "Path", "Definition")) {
    $value = $commandInfo.$property
    if ($value -and $value.Trim()) {
      return $value
    }
  }

  return $FilePath
}

function Resolve-PnpmInvocationSpec {
  param(
    [string]$ResolvedPath,
    [string[]]$ArgumentList = @()
  )

  if (-not $ResolvedPath) {
    return $null
  }

  $commandName = [System.IO.Path]::GetFileNameWithoutExtension($ResolvedPath)
  if (-not $commandName.Equals("pnpm", [System.StringComparison]::OrdinalIgnoreCase)) {
    return $null
  }

  $commandDir = Split-Path -Path $ResolvedPath -Parent
  if (-not $commandDir) {
    return $null
  }

  $nodeExecutable = Join-Path $commandDir "node.exe"
  $pnpmCliScript = Join-Path $commandDir "node_modules\pnpm\bin\pnpm.cjs"
  if (-not ((Test-Path $nodeExecutable) -and (Test-Path $pnpmCliScript))) {
    return $null
  }

  $launchArguments = @($pnpmCliScript) + $ArgumentList
  [pscustomobject]@{
    ResolvedPath = $ResolvedPath
    LaunchFilePath = $nodeExecutable
    LaunchArguments = $launchArguments
    DisplayCommand = if ($launchArguments.Count -gt 0) {
      "{0} {1}" -f $nodeExecutable, (Format-ArgumentList -ArgumentList $launchArguments)
    }
    else {
      $nodeExecutable
    }
    Strategy = "pnpm-node-direct"
  }
}

function Resolve-InvocationSpec {
  param(
    [string]$FilePath,
    [string[]]$ArgumentList = @()
  )

  $resolvedPath = Resolve-ExecutablePath -FilePath $FilePath
  $pnpmInvocation = Resolve-PnpmInvocationSpec -ResolvedPath $resolvedPath -ArgumentList $ArgumentList
  if ($pnpmInvocation) {
    return $pnpmInvocation
  }

  $launchFilePath = $resolvedPath
  $launchArguments = $ArgumentList
  $strategy = "direct"
  $extension = [System.IO.Path]::GetExtension($resolvedPath)
  if ($extension -eq ".ps1") {
    $launchFilePath = Resolve-PowerShellExecutable
    $launchArguments = @("-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $resolvedPath) + $ArgumentList
    $strategy = "powershell-script"
  }
  elseif ($extension -in @(".cmd", ".bat")) {
    $cmdLine = @((Convert-ToCmdArgument -Value $resolvedPath))
    foreach ($argument in $ArgumentList) {
      $cmdLine += (Convert-ToCmdArgument -Value $argument)
    }
    $launchFilePath = "cmd.exe"
    $launchArguments = @("/d", "/c", ($cmdLine -join " "))
    $strategy = "cmd-wrapper"
  }

  [pscustomobject]@{
    ResolvedPath = $resolvedPath
    LaunchFilePath = $launchFilePath
    LaunchArguments = $launchArguments
    DisplayCommand = if ($launchArguments.Count -gt 0) {
      "{0} {1}" -f $launchFilePath, (Format-ArgumentList -ArgumentList $launchArguments)
    }
    else {
      $launchFilePath
    }
    Strategy = $strategy
  }
}

function Invoke-Step {
  param(
    [string]$Label,
    [string]$FilePath,
    [string[]]$ArgumentList = @(),
    [string]$WorkingDirectory,
    [string]$LogPath
  )

  $displayLabel = if ($Label) { $Label } else { $FilePath }
  $stdoutPath = if ($LogPath) { "$LogPath.stdout.log" } else { [System.IO.Path]::GetTempFileName() }
  $stderrPath = if ($LogPath) { "$LogPath.stderr.log" } else { [System.IO.Path]::GetTempFileName() }
  $startedAt = Get-Date
  $invocation = Resolve-InvocationSpec -FilePath $FilePath -ArgumentList $ArgumentList

  Write-Host ("  -> {0}" -f $displayLabel)
  if ($WorkingDirectory) {
    Push-Location $WorkingDirectory
  }

  try {
    try {
      $process = Start-Process -FilePath $invocation.LaunchFilePath -ArgumentList $invocation.LaunchArguments -PassThru -Wait -NoNewWindow -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    }
    catch {
      $message = @(
        "Failed to start command: $displayLabel",
        "Resolved path: $($invocation.ResolvedPath)",
        "Launch command: $($invocation.DisplayCommand)",
        "Working directory: $WorkingDirectory",
        "Launch strategy: $($invocation.Strategy)",
        "Error: $($_.Exception.Message)"
      ) -join [Environment]::NewLine
      throw $message
    }

    $process.Refresh()

    if ($process.ExitCode -ne 0) {
      Show-CommandFailureLog -StdoutPath $stdoutPath -StderrPath $stderrPath
      $message = @(
        "Command failed: $displayLabel",
        "Exit code: $($process.ExitCode)",
        "Resolved path: $($invocation.ResolvedPath)",
        "Launch command: $($invocation.DisplayCommand)",
        "Working directory: $WorkingDirectory",
        "Launch strategy: $($invocation.Strategy)",
        "Stdout log: $stdoutPath",
        "Stderr log: $stderrPath"
      ) -join [Environment]::NewLine
      throw $message
    }

    $elapsed = [math]::Round(((Get-Date) - $startedAt).TotalSeconds, 1)
    Write-Host ("  [OK] {0} ({1}s)" -f $displayLabel, $elapsed)
  }
  finally {
    if ($WorkingDirectory) {
      Pop-Location
    }

    if (-not $LogPath) {
      Remove-Item -Path $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue
    }
  }
}

#endregion

#region Prerequisite and filesystem helpers

function Assert-CommandAvailable {
  param(
    [string]$CommandName,
    [string]$InstallHint
  )

  if (-not (Get-Command $CommandName -ErrorAction SilentlyContinue)) {
    throw "Missing required command '$CommandName'. $InstallHint"
  }
}

function Resolve-PowerShellExecutable {
  foreach ($commandName in @("pwsh", "powershell")) {
    if (Get-Command $commandName -ErrorAction SilentlyContinue) {
      return $commandName
    }
  }

  throw "Missing required PowerShell host. Install PowerShell 7 (`"pwsh`") or Windows PowerShell (`"powershell`")."
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

#endregion

#region Browser, package, and configuration helpers

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

#endregion

#region Process, persistence, and service helpers

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

function Stop-ProcessIdQuietly {
  param([int]$ProcessId)

  if ($ProcessId -eq $PID) {
    return
  }

  try {
    Stop-Process -Id $ProcessId -Force -ErrorAction SilentlyContinue
    Wait-Process -Id $ProcessId -Timeout 3 -ErrorAction SilentlyContinue
  }
  catch {
  }
}

function Test-PathFragmentMatch {
  param(
    [string]$Haystack,
    [string[]]$Needles
  )

  if (-not $Haystack) {
    return $false
  }

  $normalizedHaystack = $Haystack.Replace("/", "\")
  foreach ($needle in $Needles) {
    if ($needle -and $normalizedHaystack.IndexOf($needle, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
      return $true
    }
  }

  return $false
}

function Stop-TablinqRuntimeProcesses {
  param(
    [string]$InstallRoot,
    [string]$StateRoot,
    [string]$RepoRoot
  )

  $installRootPath = [System.IO.Path]::GetFullPath($InstallRoot)
  $stateRootPath = [System.IO.Path]::GetFullPath($StateRoot)
  $repoRootPath = [System.IO.Path]::GetFullPath($RepoRoot)
  $targetRootPath = [System.IO.Path]::GetFullPath((Join-Path $repoRootPath "target"))
  $pathFragments = @($installRootPath, $stateRootPath, $repoRootPath, $targetRootPath)
  $browserAdapterFragments = @(
    "\packages\browser-adapter\src\service.ts",
    "\packages\browser-adapter\dist\src\service.js",
    "\browser-adapter\src\service.js",
    "\browser-adapter\src\service.ts",
    "\browser-adapter\dist\src\service.js"
  )
  $runtimeNames = @(
    "gpt.exe",
    "tablinq-cli.exe",
    "gptd.exe",
    "tablinq-gateway.exe",
    "tablinq-host.exe"
  )
  $stopped = [System.Collections.Generic.List[string]]::new()
  $processes = Get-CimInstance Win32_Process -ErrorAction SilentlyContinue
  foreach ($process in $processes) {
    $name = $process.Name
    $commandLine = [string]$process.CommandLine
    $executablePath = [string]$process.ExecutablePath
    $isRuntimeBinary =
      $runtimeNames -contains $name -and
      (
        (Test-PathFragmentMatch -Haystack $commandLine -Needles $pathFragments) -or
        (Test-PathFragmentMatch -Haystack $executablePath -Needles $pathFragments)
      )
    $isBrowserAdapter =
      $name -eq "node.exe" -and
      (
        (Test-PathFragmentMatch -Haystack $commandLine -Needles $browserAdapterFragments) -or
        (
          (Test-PathFragmentMatch -Haystack $commandLine -Needles @("tablinq")) -and
          (Test-PathFragmentMatch -Haystack $commandLine -Needles @("browser-adapter"))
        )
      )

    if (-not $isRuntimeBinary -and -not $isBrowserAdapter) {
      continue
    }

    $stopped.Add(("{0}({1})" -f $name, $process.ProcessId)) | Out-Null
    Stop-ProcessIdQuietly -ProcessId $process.ProcessId
  }

  Stop-ManagedBrowserProfileProcesses -StateRoot $StateRoot

  if ($stopped.Count -gt 0) {
    Write-Host ("  stopped: {0}" -f ($stopped -join ", "))
  }
  else {
    Write-Host "  stopped: no existing Tablinq runtime processes"
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

function Invoke-InternalCli {
  param(
    [string]$CliExecutablePath,
    [string]$StateRoot,
    [string[]]$Arguments,
    [int]$TimeoutSeconds = 15
  )

  $stdoutPath = [System.IO.Path]::GetTempFileName()
  $stderrPath = [System.IO.Path]::GetTempFileName()
  try {
    $process = Start-Process -FilePath $CliExecutablePath -ArgumentList (@("--state-root", $StateRoot) + $Arguments) -PassThru -NoNewWindow -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while (-not $process.HasExited -and (Get-Date) -lt $deadline) {
      Start-Sleep -Milliseconds 100
      $process.Refresh()
    }

    if (-not $process.HasExited) {
      Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue
      throw "Command timed out: $($Arguments -join ' ')"
    }

    $process.WaitForExit()
    $process.Refresh()

    $stdout = Get-Content -Path $stdoutPath -Raw -ErrorAction SilentlyContinue
    $stderr = Get-Content -Path $stderrPath -Raw -ErrorAction SilentlyContinue
    $stdoutText = if ($null -ne $stdout) { ($stdout | Out-String).Trim() } else { "" }
    $stderrText = if ($null -ne $stderr) { $stderr.Trim() } else { "" }
    [pscustomobject]@{
      ExitCode = $process.ExitCode
      Stdout = $stdoutText
      Stderr = $stderrText
    }
  }
  finally {
    Remove-Item -Path $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue
  }
}

function Invoke-InternalCliJson {
  param(
    [string]$CliExecutablePath,
    [string]$StateRoot,
    [string[]]$Arguments
  )

  $result = Invoke-InternalCli -CliExecutablePath $CliExecutablePath -StateRoot $StateRoot -Arguments $Arguments
  if ($result.ExitCode -ne 0) {
    $message = if ($result.Stderr) { $result.Stderr } elseif ($result.Stdout) { $result.Stdout } else { "Command failed: $($Arguments -join ' ')" }
    throw $message
  }

  if (-not $result.Stdout) {
    return $null
  }

  try {
    return ($result.Stdout | ConvertFrom-Json -Depth 20)
  }
  catch {
    throw "Command returned non-JSON output: $($Arguments -join ' ')"
  }
}

function Get-DaemonStatus {
  param(
    [string]$CliExecutablePath,
    [string]$StateRoot
  )

  $result = Invoke-InternalCli -CliExecutablePath $CliExecutablePath -StateRoot $StateRoot -Arguments @("daemon", "status")
  if ($result.ExitCode -ne 0 -or -not $result.Stdout) {
    return $null
  }

  try {
    return ($result.Stdout | ConvertFrom-Json -Depth 20)
  }
  catch {
    return $null
  }
}

function Get-GatewayStatus {
  param(
    [string]$CliExecutablePath,
    [string]$StateRoot
  )

  $result = Invoke-InternalCli -CliExecutablePath $CliExecutablePath -StateRoot $StateRoot -Arguments @("gateway", "status")
  if ($result.ExitCode -ne 0 -or -not $result.Stdout) {
    return $null
  }

  try {
    return ($result.Stdout | ConvertFrom-Json -Depth 20)
  }
  catch {
    return $null
  }
}

function Wait-ForDaemonReady {
  param(
    [string]$CliExecutablePath,
    [string]$StateRoot,
    [int]$TimeoutSeconds = 10
  )

  $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
  while ((Get-Date) -lt $deadline) {
    $status = Get-DaemonStatus -CliExecutablePath $CliExecutablePath -StateRoot $StateRoot
    if ($status -and $status.type -eq "status" -and $status.payload) {
      return $status
    }
    Start-Sleep -Milliseconds 250
  }

  throw "Daemon did not become ready within $TimeoutSeconds seconds."
}

function Wait-ForGatewayReady {
  param(
    [string]$CliExecutablePath,
    [string]$StateRoot,
    [int]$TimeoutSeconds = 10
  )

  $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
  while ((Get-Date) -lt $deadline) {
    $status = Get-GatewayStatus -CliExecutablePath $CliExecutablePath -StateRoot $StateRoot
    if ($status -and $status.running -eq $true -and $status.ready -eq $true) {
      return $status
    }
    Start-Sleep -Milliseconds 250
  }

  throw "Gateway did not become ready within $TimeoutSeconds seconds."
}

#endregion

#region Reporting helpers

function Get-ShortCommit {
  param([string]$Commit)

  if (-not $Commit) {
    return "unknown"
  }

  if ($Commit.Length -le 12) {
    return $Commit
  }

  return $Commit.Substring(0, 12)
}

function Format-BuildMetadataSummary {
  param([pscustomobject]$Metadata)

  if (-not $Metadata) {
    return "unavailable"
  }

  $commit = Get-ShortCommit -Commit $Metadata.git_commit
  "version={0} commit={1} dirty={2} profile={3}" -f $Metadata.version, $commit, $Metadata.git_dirty, $Metadata.profile
}

function Resolve-CommandPath {
  param([string]$CommandName)

  $commandInfo = Get-Command $CommandName -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $commandInfo) {
    return $null
  }

  foreach ($property in @("Source", "Path", "Definition")) {
    $value = $commandInfo.$property
    if ($value -and $value.Trim()) {
      return $value
    }
  }

  return $null
}

function Get-PackageMetadataPathForCli {
  param([string]$ExecutablePath)

  if (-not $ExecutablePath) {
    return $null
  }

  $binDirectory = Split-Path -Path $ExecutablePath -Parent
  if (-not $binDirectory) {
    return $null
  }

  $installRoot = Split-Path -Path $binDirectory -Parent
  if (-not $installRoot) {
    return $null
  }

  $candidate = Join-Path $installRoot "package-metadata.json"
  if (Test-Path $candidate) {
    return $candidate
  }

  return $null
}

function Get-BuildMetadataFromPackage {
  param([string]$ExecutablePath)

  $packageMetadataPath = Get-PackageMetadataPathForCli -ExecutablePath $ExecutablePath
  if (-not $packageMetadataPath) {
    return $null
  }

  try {
    $packageMetadata = Get-Content -Path $packageMetadataPath -Raw | ConvertFrom-Json
    [pscustomobject]@{
      application = "gpt"
      version = $packageMetadata.package_version
      git_commit = $packageMetadata.commit
      git_dirty = [bool]$packageMetadata.dirty
      target = $packageMetadata.target
      profile = $packageMetadata.profile
      built_at_unix_epoch = [uint64]$packageMetadata.built_at_unix_epoch
    }
  }
  catch {
    return $null
  }
}

function Get-BuildMetadata {
  param(
    [string]$ExecutablePath,
    [string]$WorkingDirectory
  )

  if (-not $ExecutablePath) {
    return $null
  }

  $packageMetadata = Get-BuildMetadataFromPackage -ExecutablePath $ExecutablePath
  if ($packageMetadata) {
    return $packageMetadata
  }

  try {
    if ($WorkingDirectory) {
      Push-Location $WorkingDirectory
    }

    try {
      $output = & $ExecutablePath version --json 2>&1
      $exitCode = $LASTEXITCODE
    }
    finally {
      if ($WorkingDirectory) {
        Pop-Location
      }
    }

    $text = ($output | Out-String).Trim()
    if ($exitCode -ne 0 -or -not $text) {
      return $null
    }

    return ($text | ConvertFrom-Json -Depth 10)
  }
  catch {
    return $null
  }
}

function Get-WorkspaceHeadCommit {
  param([string]$RepoRoot)

  try {
    $output = & git -C $RepoRoot rev-parse HEAD 2>$null
    $exitCode = $LASTEXITCODE
    $value = ($output | Out-String).Trim()
    if ($exitCode -ne 0 -or -not $value) {
      return $null
    }

    return $value
  }
  catch {
    return $null
  }
}

function Get-InstallVersionDiagnostics {
  param(
    [string]$InstalledCli,
    [string]$RepoRoot
  )

  $installedMetadata = Get-BuildMetadata -ExecutablePath $InstalledCli -WorkingDirectory $RepoRoot
  $pathCommand = Resolve-CommandPath -CommandName "gpt"
  $pathMetadata = Get-BuildMetadata -ExecutablePath $pathCommand -WorkingDirectory $RepoRoot
  $workspaceHead = Get-WorkspaceHeadCommit -RepoRoot $RepoRoot
  $warnings = [System.Collections.Generic.List[string]]::new()

  $pathState = "missing"
  $pathDetail = "gpt is not resolved in this shell"
  if ($pathCommand) {
    $sameBinary = $pathCommand.Trim().Equals($InstalledCli, [System.StringComparison]::OrdinalIgnoreCase)
    $sameCommit = $installedMetadata -and $pathMetadata -and $installedMetadata.git_commit -and $pathMetadata.git_commit -and $installedMetadata.git_commit.Equals($pathMetadata.git_commit, [System.StringComparison]::OrdinalIgnoreCase)
    if ($sameBinary -or $sameCommit) {
      $pathState = "current"
      $pathDetail = "{0} {1}" -f $pathCommand, (Format-BuildMetadataSummary -Metadata $pathMetadata)
    }
    else {
      $pathState = "stale"
      $pathDetail = "{0} {1}" -f $pathCommand, (Format-BuildMetadataSummary -Metadata $pathMetadata)
      $warnings.Add(
        "Current shell resolves gpt to $pathCommand, but the freshly installed CLI is $InstalledCli. Open a new shell or use the installed path for this session."
      ) | Out-Null
    }
  }

  $workspaceState = "unknown"
  $workspaceDetail = "git HEAD unavailable"
  if ($workspaceHead) {
    $workspaceDetail = "HEAD={0}" -f (Get-ShortCommit -Commit $workspaceHead)
    if ($installedMetadata -and $installedMetadata.git_commit) {
      if ($installedMetadata.git_commit.Equals($workspaceHead, [System.StringComparison]::OrdinalIgnoreCase)) {
        $workspaceState = "matching"
      }
      else {
        $workspaceState = "different"
        $warnings.Add(
          "Repo checkout HEAD $(Get-ShortCommit -Commit $workspaceHead) differs from installed CLI commit $(Get-ShortCommit -Commit $installedMetadata.git_commit). Use cargo run -p gpt -- ... when validating checkout-only changes."
        ) | Out-Null
      }
    }
  }

  [pscustomobject]@{
    InstalledMetadata = $installedMetadata
    PathCommand = $pathCommand
    PathMetadata = $pathMetadata
    PathState = $pathState
    PathDetail = $pathDetail
    WorkspaceState = $workspaceState
    WorkspaceDetail = $workspaceDetail
    Warnings = $warnings
  }
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

function Write-ManualSteps {
  param(
    [string]$InstallRoot,
    [string]$StateRoot,
    [string]$BrowserName,
    [string]$ConnectionMode,
    [string]$TargetUrl,
    [bool]$DaemonRunning,
    [bool]$GatewayRunning,
    [bool]$UsePathCommand
  )

  $extensionPath = Join-Path $InstallRoot "extension"
  $chromeAttachProfilePath = Join-Path $StateRoot "chrome-attach-profile"
  $gptCommand = if ($UsePathCommand) { "gpt" } else { Join-Path $InstallRoot "bin\gpt.exe" }
  $steps = [System.Collections.Generic.List[string]]::new()

  if (-not $DaemonRunning) {
    $steps.Add("$gptCommand daemon start") | Out-Null
  }

  if (-not $GatewayRunning) {
    $steps.Add("$gptCommand gateway start") | Out-Null
  }

  switch ($ConnectionMode) {
    "managed_only" {
      $steps.Add("$gptCommand browser open") | Out-Null
      $steps.Add("Sign in at $TargetUrl if prompted.") | Out-Null
    }
    "managed_fallback" {
      $steps.Add("$gptCommand browser open") | Out-Null
      $steps.Add("Sign in at $TargetUrl if prompted.") | Out-Null
    }
    default {
      if ($BrowserName -eq "Chrome") {
        $steps.Add("Fully close Chrome: taskkill /IM chrome.exe /F") | Out-Null
        $steps.Add("$gptCommand browser start-debug") | Out-Null
        $steps.Add("In that Chrome window, open chrome://extensions/ and enable Developer mode. If Tablinq is missing or disabled, load unpacked from: $extensionPath") | Out-Null
        $steps.Add("Sign in at $TargetUrl in that profile.") | Out-Null
      }
      else {
        $steps.Add("Open edge://extensions/ and enable Developer mode.") | Out-Null
        $steps.Add("If Tablinq is missing or disabled, load unpacked from: $extensionPath") | Out-Null
        $steps.Add("Fully close Edge.") | Out-Null
        $steps.Add("$gptCommand browser start-debug") | Out-Null
        $steps.Add("Sign in at $TargetUrl in that profile.") | Out-Null
      }

      $steps.Add("$gptCommand browser attach") | Out-Null
    }
  }

  Write-Host ""
  if ($ConnectionMode -eq "attach_first" -and $BrowserName -eq "Chrome") {
    Write-Host "Chrome attach-first uses the dedicated profile created automatically at: $chromeAttachProfilePath"
    Write-Host ""
  }

  Write-Host "Manual steps:"

  for ($index = 0; $index -lt $steps.Count; $index += 1) {
    Write-Host (("{0}. {1}" -f ($index + 1), $steps[$index]))
  }

  if ($ConnectionMode -eq "managed_only" -or $ConnectionMode -eq "managed_fallback") {
    Write-Host ""
    Write-Host "Managed browser mode is a fallback path. Use attach-first unless your machine or browser profile cannot expose the local debug endpoint."
  }

  Write-Host ""
  Write-Host "Optional checks:"
  Write-Host ("- {0} browser connection-info" -f $gptCommand)
  Write-Host ("- {0} status" -f $gptCommand)
  Write-Host ("- {0} doctor" -f $gptCommand)
  Write-Host ("- {0} capabilities" -f $gptCommand)
  Write-Host ("- {0} ask `"hi`"" -f $gptCommand)
}

#endregion

#region Install phases

function Assert-InstallPrerequisites {
  param(
    [bool]$ShouldBuildPackage,
    [bool]$ShouldInstallDependencies,
    [bool]$SkipRegistry,
    [string]$InstallRoot,
    [string]$StateRoot
  )

  Start-InstallPhase -Title "Checking prerequisites"
  if ($ShouldBuildPackage) {
    Assert-CommandAvailable -CommandName "cargo" -InstallHint "Install Rust and make sure cargo is on PATH."
    Assert-CommandAvailable -CommandName "node" -InstallHint "Install Node.js 20 or newer and make sure node is on PATH."
    Assert-CommandAvailable -CommandName "pnpm" -InstallHint "Install pnpm and make sure it is on PATH."
  }
  elseif ($ShouldInstallDependencies) {
    Assert-CommandAvailable -CommandName "pnpm" -InstallHint "Install pnpm and make sure it is on PATH."
  }

  $powerShellExecutable = Resolve-PowerShellExecutable
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
  Ensure-StateLayout -StateRoot $StateRoot

  if (-not $SkipRegistry) {
    Assert-RegistryWritable -KeyPath "HKCU\Software"
  }

  [pscustomobject]@{
    PowerShellExecutable = $powerShellExecutable
    BrowserSelection = $browserSelection
    InstallLogDir = Join-Path $StateRoot "logs"
  }
}

function Invoke-WorkspaceDependencyInstall {
  param(
    [bool]$ShouldInstallDependencies,
    [string]$RepoRoot,
    [string]$InstallLogDir
  )

  if (-not $ShouldInstallDependencies) {
    return
  }

  Start-InstallPhase -Title "Installing workspace dependencies"
  Invoke-Step -Label "pnpm install" -FilePath "pnpm" -ArgumentList @("install") -WorkingDirectory $RepoRoot -LogPath (Join-Path $InstallLogDir "install-pnpm")
}

function Invoke-PackageBuildStep {
  param(
    [bool]$ShouldBuildPackage,
    [string]$PowerShellExecutable,
    [string]$PackageScriptPath,
    [string]$Profile,
    [string]$RepoRoot,
    [string]$InstallLogDir
  )

  if (-not $ShouldBuildPackage) {
    return
  }

  Start-InstallPhase -Title "Building package"
  Invoke-Step -FilePath $PowerShellExecutable -ArgumentList @(
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    $PackageScriptPath,
    "-Profile",
    $Profile
  ) -WorkingDirectory $RepoRoot -Label "package.ps1" -LogPath (Join-Path $InstallLogDir "install-package")
}

function Install-PackagePayload {
  param(
    [string]$PackageRoot,
    [string]$InstallRoot,
    [string]$StateRoot,
    [string]$RepoRoot,
    [pscustomobject]$BrowserSelection,
    [bool]$SkipRegistry,
    [bool]$SkipPathUpdate
  )

  if (-not (Test-Path $PackageRoot)) {
    throw "Missing package root after packaging: $PackageRoot"
  }

  Start-InstallPhase -Title "Replacing install directory"
  $existingBinDir = Join-Path $InstallRoot "bin"
  Stop-ManagedProcessByPath -ExecutablePath (Join-Path $existingBinDir "tablinq-gateway.exe")
  Stop-ManagedProcessByPath -ExecutablePath (Join-Path $existingBinDir "gptd.exe")
  Stop-TablinqRuntimeProcesses -InstallRoot $InstallRoot -StateRoot $StateRoot -RepoRoot $RepoRoot
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

  Set-TomlStringValue -Path $configPath -Key "browser_path" -Value $BrowserSelection.Path
  Set-TomlStringValue -Path $configPath -Key "browser_channel" -Value $BrowserSelection.ConfigChannel
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

  [pscustomobject]@{
    ConnectionMode = $connectionMode
    TargetUrl = $targetUrl
    InstalledCli = Join-Path $InstallRoot "bin\gpt.exe"
    UsePathCommand = -not $SkipPathUpdate
  }
}

function Set-LocalMachineConfiguration {
  param(
    [bool]$EnableAutoStart,
    [string]$InstallRoot,
    [string]$StateRoot
  )

  Start-InstallPhase -Title "Configuring local machine"
  Remove-AutoStartPersistence

  $autoStartMode = "disabled"
  if ($EnableAutoStart) {
    $autoStartMode = Install-AutoStartPersistence -DaemonExecutable (Join-Path $InstallRoot "bin\gptd.exe") -GatewayExecutable (Join-Path $InstallRoot "bin\tablinq-gateway.exe") -StateRoot $StateRoot -InstallRoot $InstallRoot
  }

  $autoStartMode
}

function Get-InstallServiceState {
  param(
    [string]$InstalledCli,
    [string]$InstallRoot,
    [string]$StateRoot,
    [string]$RepoRoot,
    [bool]$NoLaunchAfterInstall
  )

  $daemonStatus = $null
  $gatewayStatus = $null
  $launchMode = "skipped"

  if (-not $NoLaunchAfterInstall) {
    Start-InstallPhase -Title "Starting services"
    Stop-TablinqRuntimeProcesses -InstallRoot $InstallRoot -StateRoot $StateRoot -RepoRoot $RepoRoot
    $null = Invoke-InternalCliJson -CliExecutablePath $InstalledCli -StateRoot $StateRoot -Arguments @("daemon", "start")
    $daemonStatus = Wait-ForDaemonReady -CliExecutablePath $InstalledCli -StateRoot $StateRoot
    $null = Invoke-InternalCliJson -CliExecutablePath $InstalledCli -StateRoot $StateRoot -Arguments @("gateway", "start")
    $gatewayStatus = Wait-ForGatewayReady -CliExecutablePath $InstalledCli -StateRoot $StateRoot
    $launchMode = "started"
  }
  else {
    $daemonStatus = Get-DaemonStatus -CliExecutablePath $InstalledCli -StateRoot $StateRoot
    $gatewayStatus = Get-GatewayStatus -CliExecutablePath $InstalledCli -StateRoot $StateRoot
  }

  [pscustomobject]@{
    DaemonStatus = $daemonStatus
    GatewayStatus = $gatewayStatus
    LaunchMode = $launchMode
  }
}

function Write-InstallSummary {
  param(
    [string]$InstallRoot,
    [string]$StateRoot,
    [pscustomobject]$BrowserSelection,
    [string]$ConnectionMode,
    [string]$TargetUrl,
    [string]$PathState,
    [string]$AutoStartMode,
    [string]$LaunchMode,
    [bool]$NoLaunchAfterInstall,
    [pscustomobject]$DaemonStatus,
    [pscustomobject]$GatewayStatus,
    [bool]$EnableAutoStart,
    [bool]$SkipPathUpdate,
    [bool]$UsePathCommand,
    [pscustomobject]$VersionDiagnostics
  )

  $daemonRunning = [bool]($DaemonStatus -and $DaemonStatus.type -eq "status" -and $DaemonStatus.payload)
  $gatewayRunning = [bool]($GatewayStatus -and $GatewayStatus.running -eq $true -and $GatewayStatus.ready -eq $true)
  $daemonDetail = if ($daemonRunning) {
    $DaemonStatus.payload.ipc_endpoint
  }
  elseif ($NoLaunchAfterInstall) {
    "start later with gpt daemon start"
  }
  else {
    "not ready"
  }
  $gatewayDetail = if ($gatewayRunning) {
    "{0}:{1}" -f $GatewayStatus.host, $GatewayStatus.port
  }
  elseif ($NoLaunchAfterInstall) {
    "start later with gpt gateway start"
  }
  else {
    "not ready"
  }

  Complete-InstallProgress
  Write-Host ""
  Write-Host "Install complete."
  Write-Host ""
  Write-Host "Summary:"
  Write-StatusLine -Label "Install" -State "ok" -Details $InstallRoot
  Write-StatusLine -Label "State" -State "ok" -Details $StateRoot
  Write-StatusLine -Label "Browser" -State $BrowserSelection.Name -Details $ConnectionMode
  Write-StatusLine -Label "PATH" -State $PathState -Details $(if ($SkipPathUpdate) { "gpt not added to PATH" } else { "gpt available in new shells" })
  Write-StatusLine -Label "Autostart" -State $AutoStartMode -Details $(if ($EnableAutoStart) { "starts on login" } else { "disabled" })
  Write-StatusLine -Label "Launch" -State $LaunchMode -Details $(if ($NoLaunchAfterInstall) { "use manual steps below" } else { "daemon and gateway checked" })
  Write-StatusLine -Label "Daemon" -State $(if ($daemonRunning) { "running" } else { "attention" }) -Details $daemonDetail
  Write-StatusLine -Label "Gateway" -State $(if ($gatewayRunning) { "running" } else { "attention" }) -Details $gatewayDetail
  Write-StatusLine -Label "CLI build" -State $(if ($VersionDiagnostics.InstalledMetadata) { "ok" } else { "attention" }) -Details ("{0} {1}" -f (Join-Path $InstallRoot "bin\gpt.exe"), (Format-BuildMetadataSummary -Metadata $VersionDiagnostics.InstalledMetadata))
  Write-StatusLine -Label "PATH gpt" -State $VersionDiagnostics.PathState -Details $VersionDiagnostics.PathDetail
  Write-StatusLine -Label "Workspace" -State $VersionDiagnostics.WorkspaceState -Details $VersionDiagnostics.WorkspaceDetail

  if ($VersionDiagnostics.Warnings.Count -gt 0) {
    Write-Host ""
    Write-Host "Developer checks:"
    foreach ($warning in $VersionDiagnostics.Warnings) {
      Write-Host ("- {0}" -f $warning)
    }
  }

  Write-ManualSteps -InstallRoot $InstallRoot -StateRoot $StateRoot -BrowserName $BrowserSelection.Name -ConnectionMode $ConnectionMode -TargetUrl $TargetUrl -DaemonRunning $daemonRunning -GatewayRunning $gatewayRunning -UsePathCommand $UsePathCommand
}

#endregion

Write-Host "Installing Tablinq on Windows..."
Write-Host "Defaults: launch now = yes, autostart on login = no"
if ($usePrebuiltPackage) {
  Write-Host ("Package source: prebuilt bundle ({0})" -f $PackageRoot)
}

$preflight = Assert-InstallPrerequisites -ShouldBuildPackage:(-not $effectiveSkipPackageBuild) -ShouldInstallDependencies:(-not $effectiveSkipDependencyInstall) -SkipRegistry:([bool]$SkipRegistry) -InstallRoot $InstallRoot -StateRoot $StateRoot
Start-InstallPhase -Title "Stopping existing Tablinq runtime"
Stop-TablinqRuntimeProcesses -InstallRoot $InstallRoot -StateRoot $StateRoot -RepoRoot $repoRoot
Invoke-WorkspaceDependencyInstall -ShouldInstallDependencies:(-not $effectiveSkipDependencyInstall) -RepoRoot $repoRoot -InstallLogDir $preflight.InstallLogDir
Invoke-PackageBuildStep -ShouldBuildPackage:(-not $effectiveSkipPackageBuild) -PowerShellExecutable $preflight.PowerShellExecutable -PackageScriptPath $packageScriptPath -Profile $Profile -RepoRoot $repoRoot -InstallLogDir $preflight.InstallLogDir

$packageInstall = Install-PackagePayload -PackageRoot $PackageRoot -InstallRoot $InstallRoot -StateRoot $StateRoot -RepoRoot $repoRoot -BrowserSelection $preflight.BrowserSelection -SkipRegistry:([bool]$SkipRegistry) -SkipPathUpdate:([bool]$SkipPathUpdate)
$autoStartMode = Set-LocalMachineConfiguration -EnableAutoStart:([bool]$EnableAutoStart) -InstallRoot $InstallRoot -StateRoot $StateRoot
$serviceState = Get-InstallServiceState -InstalledCli $packageInstall.InstalledCli -InstallRoot $InstallRoot -StateRoot $StateRoot -RepoRoot $repoRoot -NoLaunchAfterInstall:([bool]$NoLaunchAfterInstall)
$pathState = if ($SkipPathUpdate) { "unchanged" } else { "updated" }
$versionDiagnostics = Get-InstallVersionDiagnostics -InstalledCli $packageInstall.InstalledCli -RepoRoot $repoRoot

Write-InstallSummary -InstallRoot $InstallRoot -StateRoot $StateRoot -BrowserSelection $preflight.BrowserSelection -ConnectionMode $packageInstall.ConnectionMode -TargetUrl $packageInstall.TargetUrl -PathState $pathState -AutoStartMode $autoStartMode -LaunchMode $serviceState.LaunchMode -NoLaunchAfterInstall:([bool]$NoLaunchAfterInstall) -DaemonStatus $serviceState.DaemonStatus -GatewayStatus $serviceState.GatewayStatus -EnableAutoStart:([bool]$EnableAutoStart) -SkipPathUpdate:([bool]$SkipPathUpdate) -UsePathCommand:([bool]$packageInstall.UsePathCommand) -VersionDiagnostics $versionDiagnostics
