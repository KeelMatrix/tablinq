param(
    [string]$EnvFile,
    [string]$PythonExe = "python",
    [string]$BaseUrl,
    [string]$LogFile,
    [string]$GptBin = "gpt",
    [switch]$NoTxt,
    [double]$BetweenPromptSleep = 10,
    [double]$InterGetSleep = 0.6,
    [double]$CancelDelay = 1.5,
    [double]$ConcurrencyGap = 1.0,
    [switch]$SkipCliAttach,
    [switch]$SkipDetachRestore,
    [switch]$ExitZeroOnWarnings
)

$ErrorActionPreference = "Stop"

function Write-Step {
    param([Parameter(Mandatory = $true)][string]$Message)
    $stamp = Get-Date -Format "HH:mm:ss"
    Write-Host "[$stamp] PS1: $Message"
}

function Load-EnvLocal {
    param([Parameter(Mandatory = $true)][string]$Path)

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        throw ".env.local was not found: $Path"
    }

    $loaded = 0
    foreach ($rawLine in Get-Content -LiteralPath $Path -ErrorAction Stop) {
        $line = $rawLine.Trim()
        if ($line.Length -eq 0) { continue }
        if ($line.StartsWith("#")) { continue }
        if ($line.StartsWith("export ")) { $line = $line.Substring(7).Trim() }

        $idx = $line.IndexOf("=")
        if ($idx -le 0) { continue }

        $key = $line.Substring(0, $idx).Trim()
        $value = $line.Substring($idx + 1).Trim()

        if ($value.Length -ge 2) {
            if (($value.StartsWith('"') -and $value.EndsWith('"')) -or ($value.StartsWith("'") -and $value.EndsWith("'"))) {
                $value = $value.Substring(1, $value.Length - 2)
            }
        }

        if ($key.Length -gt 0) {
            [Environment]::SetEnvironmentVariable($key, $value, "Process")
            $loaded += 1
        }
    }

    return $loaded
}

function Get-FirstEnvName {
    param([Parameter(Mandatory = $true)][string[]]$Names)

    foreach ($name in $Names) {
        $value = [Environment]::GetEnvironmentVariable($name, "Process")
        if (-not [string]::IsNullOrWhiteSpace($value)) {
            return $name
        }
    }

    return $null
}

$exitCode = 1
$resolvedLogFile = $null

try {
    $Host.UI.RawUI.WindowTitle = "Tablinq Step 4 Robustness Smoke"

    $scriptDir = Split-Path -Parent $PSCommandPath
    if ([string]::IsNullOrWhiteSpace($scriptDir)) {
        $scriptDir = (Get-Location).Path
    }

    Write-Step "Script directory: $scriptDir"

    if ([string]::IsNullOrWhiteSpace($EnvFile)) {
        $EnvFile = Join-Path $scriptDir ".env.local"
    }
    $EnvFile = [System.IO.Path]::GetFullPath($EnvFile)

    if ([string]::IsNullOrWhiteSpace($LogFile)) {
        $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
        $LogFile = Join-Path $scriptDir "tablinq-step4-robustness-$stamp.md"
    }
    $resolvedLogFile = [System.IO.Path]::GetFullPath($LogFile)

    Write-Step "Loading .env.local: $EnvFile"
    $loadedCount = Load-EnvLocal -Path $EnvFile
    Write-Step "Loaded $loadedCount environment value(s)"

    $apiKeyName = Get-FirstEnvName -Names @(
        "TABLINQ_GATEWAY_API_KEY",
        "TABLINQ_API_KEY",
        "GATEWAY_API_KEY",
        "OPENAI_API_KEY",
        "API_KEY"
    )
    if ($null -eq $apiKeyName) {
        throw "No API key found in .env.local. Expected one of: TABLINQ_GATEWAY_API_KEY, TABLINQ_API_KEY, GATEWAY_API_KEY, OPENAI_API_KEY, API_KEY."
    }
    Write-Step "API key source: $apiKeyName"

    $installBin = Join-Path $env:LOCALAPPDATA "Tablinq\install\bin"
    if (Test-Path -LiteralPath $installBin -PathType Container) {
        $pathParts = @($env:PATH -split ';' | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
        if ($pathParts -notcontains $installBin) {
            $env:PATH = "$installBin;$env:PATH"
            Write-Step "Prepended Tablinq install bin to PATH: $installBin"
        }
    }

    $pythonCommand = Get-Command $PythonExe -ErrorAction Stop
    Write-Step "Python resolved to: $($pythonCommand.Source)"

    $gptCommand = Get-Command $GptBin -ErrorAction SilentlyContinue
    if ($null -eq $gptCommand) {
        Write-Step "gpt command is not resolvable; Python runner will capture the exact CLI error"
    }
    else {
        Write-Step "gpt resolved to: $($gptCommand.Source)"
    }

    $pythonScript = Join-Path $scriptDir "tablinq_step4_robustness.py"
    if (-not (Test-Path -LiteralPath $pythonScript -PathType Leaf)) {
        throw "Python script was not found: $pythonScript"
    }

    $env:PYTHONUNBUFFERED = "1"

    $argsList = @(
        $pythonScript,
        "--env-file", $EnvFile,
        "--log-file", $resolvedLogFile,
        "--gpt-bin", $GptBin,
        "--between-prompt-sleep", $BetweenPromptSleep,
        "--inter-get-sleep", $InterGetSleep,
        "--cancel-delay", $CancelDelay,
        "--concurrency-gap", $ConcurrencyGap
    )

    if (-not [string]::IsNullOrWhiteSpace($BaseUrl)) {
        $argsList += @("--base-url", $BaseUrl)
    }
    if ($NoTxt) {
        $argsList += "--notxt"
    }
    if ($SkipCliAttach) {
        $argsList += "--skip-cli-attach"
    }
    if ($SkipDetachRestore) {
        $argsList += "--skip-detach-restore"
    }
    if ($ExitZeroOnWarnings) {
        $argsList += "--exit-zero-on-warnings"
    }

    Write-Step "Required state: install.ps1 completed; Steps 1, 1.2, 2, and 3 are clean; Chrome debug browser is running"
    Write-Step "This script sends negative, upload, cancellation, concurrency, and cleanup probes"
    Write-Step "This script will NOT use tablinq-research and will NOT send deep research prompts"
    Write-Step "Prompt POST transport errors are not blindly retried"
    if ($NoTxt) {
        Write-Step "NoTxt mode enabled: upload fixture will have no .txt extension but will be sent as text/plain"
    }
    else {
        Write-Step "NoTxt mode disabled: upload fixture will use .txt extension"
    }
    Write-Step "Log file will be: $resolvedLogFile"
    Write-Step "Starting Step 4 robustness smoke"
    Write-Step "Command: $PythonExe $($argsList -join ' ')"

    & $PythonExe @argsList
    $exitCode = $LASTEXITCODE

    Write-Step "Step 4 finished with exit code $exitCode"

    if (Test-Path -LiteralPath $resolvedLogFile -PathType Leaf) {
        Write-Step "Markdown log exists: $resolvedLogFile"
        Write-Step "Log size bytes: $((Get-Item -LiteralPath $resolvedLogFile).Length)"
        Write-Step "Last 55 log lines follow"
        Write-Host ""
        Get-Content -LiteralPath $resolvedLogFile -Tail 55
        Write-Host ""
    }
    else {
        Write-Step "Markdown log was not found at expected path: $resolvedLogFile"
    }
}
catch {
    $exitCode = 1
    Write-Host ""
    Write-Step "Launcher failed"
    Write-Host $_.Exception.Message
}
finally {
    Write-Host ""
    Write-Step "Finished. Exit code: $exitCode"
    if (-not [string]::IsNullOrWhiteSpace($resolvedLogFile)) {
        Write-Step "Final log path: $resolvedLogFile"
    }
    Read-Host "Press Enter to close this window"
}

exit $exitCode
