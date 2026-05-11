param(
    [string]$EnvFile,
    [string]$PythonExe = "python",
    [string]$BaseUrl,
    [string]$LogFile,
    [int]$ConvergenceRounds = 5,
    [int]$TransportLoops = 5,
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
    $Host.UI.RawUI.WindowTitle = "Tablinq Step 1.2 Health and Transport Diagnostics"

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
        $LogFile = Join-Path $scriptDir "tablinq-step1-2-diagnostics-$stamp.md"
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

    $pythonCommand = Get-Command $PythonExe -ErrorAction Stop
    Write-Step "Python resolved to: $($pythonCommand.Source)"

    $pythonScript = Join-Path $scriptDir "tablinq_step1_2_diagnostics.py"
    if (-not (Test-Path -LiteralPath $pythonScript -PathType Leaf)) {
        throw "Python script was not found: $pythonScript"
    }

    $env:PYTHONUNBUFFERED = "1"

    $argsList = @(
        $pythonScript,
        "--env-file", $EnvFile,
        "--log-file", $resolvedLogFile,
        "--convergence-rounds", $ConvergenceRounds,
        "--transport-loops", $TransportLoops
    )

    if (-not [string]::IsNullOrWhiteSpace($BaseUrl)) {
        $argsList += @("--base-url", $BaseUrl)
    }

    if ($ExitZeroOnWarnings) {
        $argsList += "--exit-zero-on-warnings"
    }

    Write-Step "Assumption: step 1 already completed and browser is attached"
    Write-Step "Log file will be: $resolvedLogFile"
    Write-Step "Starting diagnostics"
    Write-Step "Command: $PythonExe $($argsList -join ' ')"

    & $PythonExe @argsList
    $exitCode = $LASTEXITCODE

    Write-Step "Diagnostics finished with exit code $exitCode"

    if (Test-Path -LiteralPath $resolvedLogFile -PathType Leaf) {
        Write-Step "Markdown log exists: $resolvedLogFile"
        Write-Step "Log size bytes: $((Get-Item -LiteralPath $resolvedLogFile).Length)"
        Write-Step "Last 30 log lines follow"
        Write-Host ""
        Get-Content -LiteralPath $resolvedLogFile -Tail 30
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
