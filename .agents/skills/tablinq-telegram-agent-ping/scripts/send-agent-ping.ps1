#Requires -Version 7.0

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [AllowEmptyString()]
    [string]$Text
)

$ErrorActionPreference = 'Stop'

function Exit-Fast {
    exit 0
}

try {
    $scriptDir = Split-Path -Parent $PSCommandPath
    if ([string]::IsNullOrWhiteSpace($scriptDir)) { Exit-Fast }

    $envPath = Join-Path $scriptDir '.env.local'
    if (-not (Test-Path -LiteralPath $envPath -PathType Leaf)) { Exit-Fast }

    $vars = @{}

    foreach ($line in Get-Content -LiteralPath $envPath -ErrorAction Stop) {
        $trimmed = $line.Trim()
        if ($trimmed.Length -eq 0) { continue }
        if ($trimmed.StartsWith('#')) { continue }

        $idx = $trimmed.IndexOf('=')
        if ($idx -le 0) { continue }

        $key = $trimmed.Substring(0, $idx).Trim()
        $value = $trimmed.Substring($idx + 1).Trim()

        if (
            ($value.StartsWith('"') -and $value.EndsWith('"')) -or
            ($value.StartsWith("'") -and $value.EndsWith("'"))
        ) {
            $value = $value.Substring(1, $value.Length - 2)
        }

        if ($key.Length -gt 0) {
            $vars[$key] = $value
        }
    }

    $token = $vars['TELEGRAM_BOT_TOKEN']
    $chatId = $vars['TELEGRAM_CHAT_ID']

    if ([string]::IsNullOrWhiteSpace($token)) { Exit-Fast }
    if ([string]::IsNullOrWhiteSpace($chatId)) { Exit-Fast }
    if ([string]::IsNullOrWhiteSpace($Text)) { Exit-Fast }

    $maxLen = 900
    $message = $Text.Trim()
    if ($message.Length -gt $maxLen) {
        $message = $message.Substring(0, $maxLen - 1) + '…'
    }

    $uri = "https://api.telegram.org/bot$token/sendMessage"

    $body = @{
        chat_id = $chatId
        text = $message
        disable_web_page_preview = $true
    }

    Invoke-RestMethod -Method Post -Uri $uri -Body $body -ContentType 'application/x-www-form-urlencoded' -TimeoutSec 5 | Out-Null
    exit 0
}
catch {
    exit 0
}