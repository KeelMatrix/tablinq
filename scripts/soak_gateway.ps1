param(
    [string]$BaseUrl = "http://127.0.0.1:3847/v1",
    [string]$ApiKey,
    [string]$Model = "tablinq-instant",
    [int]$Count = 10,
    [string]$Prompt = "warm browser sequential soak check"
)

$headers = @{
    "Content-Type" = "application/json"
}

if ($ApiKey) {
    $headers["Authorization"] = "Bearer $ApiKey"
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
        $response = Invoke-RestMethod -Method Post -Uri "$BaseUrl/chat/completions" -Headers $headers -Body $body -TimeoutSec 90
        $elapsed = [int]((Get-Date) - $started).TotalMilliseconds
        Write-Host "[$requestId] ok ${elapsed}ms job=$($response.tablinq_job_id) model=$($response.model)"
    } catch {
        $elapsed = [int]((Get-Date) - $started).TotalMilliseconds
        Write-Host "[$requestId] failed ${elapsed}ms $($_.Exception.Message)"
        throw
    }
}
