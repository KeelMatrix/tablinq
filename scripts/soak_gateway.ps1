param(
    [string]$BaseUrl = "http://127.0.0.1:3847/v1",
    [string]$ApiKey,
    [string]$Model = "tablinq-instant",
    [int]$Count = 10,
    [string]$Prompt = "warm browser sequential soak check",
    [switch]$CompactLog,
    [switch]$FullLog
)

if ($CompactLog -and $FullLog) {
    throw "CompactLog and FullLog cannot be combined"
}

$baseUrl = $BaseUrl.TrimEnd("/")
$headers = @{
    "Content-Type" = "application/json"
}

if ($ApiKey) {
    $headers["Authorization"] = "Bearer $ApiKey"
}

function Get-GatewayState {
    param(
        [string]$Path
    )

    Invoke-RestMethod -Method Get -Uri "$baseUrl$Path" -Headers $headers -TimeoutSec 30
}

for ($i = 1; $i -le $Count; $i++) {
    $requestId = "soak-$i"
    $headers["X-Request-Id"] = $requestId
    $body = @{
        model = $Model
        messages = @(
            @{
                role = "user"
                content = "$Prompt #$i"
            }
        )
    } | ConvertTo-Json -Depth 5

    $started = Get-Date
    try {
        $response = Invoke-RestMethod -Method Post -Uri "$baseUrl/chat/completions" -Headers $headers -Body $body -TimeoutSec 90
        $elapsed = [int]((Get-Date) - $started).TotalMilliseconds
        Write-Host "[$requestId] ok ${elapsed}ms job=$($response.tablinq_job_id) model=$($response.model)"
        if ($CompactLog) {
            $state = Get-GatewayState -Path "/requests?view=summary&limit=5"
            Write-Host "[$requestId] recent_requests=$($state.pagination.returned) has_more=$($state.pagination.has_more)"
        } elseif ($FullLog) {
            $state = Get-GatewayState -Path "/requests?view=full&limit=5"
            $state | ConvertTo-Json -Depth 20
        }
    } catch {
        $elapsed = [int]((Get-Date) - $started).TotalMilliseconds
        Write-Host "[$requestId] failed ${elapsed}ms $($_.Exception.Message)"
        if ($FullLog) {
            try {
                $state = Get-GatewayState -Path "/requests/$requestId`?view=full&include=evidence,debug,raw"
                $state | ConvertTo-Json -Depth 30
            } catch {
                Write-Host "[$requestId] full request state unavailable: $($_.Exception.Message)"
            }
        }
        throw
    }
}
