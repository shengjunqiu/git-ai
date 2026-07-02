param([string]$Server = "http://127.0.0.1:8787")

$jsonFile = Join-Path $PSScriptRoot "demo-data.json"
$records = Get-Content $jsonFile -Raw -Encoding UTF8 | ConvertFrom-Json

$wc = New-Object System.Net.WebClient
$wc.Headers.Add("Content-Type", "application/json")
$wc.Encoding = [System.Text.Encoding]::UTF8

Write-Host "`n=== Git-AI Demo Data Seeder ===" -ForegroundColor Cyan
Write-Host "Server: $Server  |  Records: $($records.Count)`n"

$ok = 0; $err = 0
foreach ($rec in $records) {
    $json = $rec | ConvertTo-Json -Depth 5 -Compress
    try {
        $resp = $wc.UploadString("$Server/api/v1/summaries", "POST", $json) | ConvertFrom-Json
        Write-Host ("  [OK ] id={0,-3} devs={1}  {2}/{3} > {4}" -f $resp.summary_id, $resp.developer_count, $rec.organization, $rec.department, $rec.project_name) -ForegroundColor Green
        $ok++
    } catch {
        Write-Host ("  [ERR] {0} > {1} : {2}" -f $rec.organization, $rec.project_name, $_.Exception.Message) -ForegroundColor Red
        $err++
    }
}

Write-Host "`n=== Done: $ok OK, $err failed ===" -ForegroundColor Cyan
Write-Host "Open: $Server" -ForegroundColor White
