[CmdletBinding()]
param(
    [string]$Ref = "",
    [switch]$Watch
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

if (-not $Ref) {
    $Ref = (& git branch --show-current).Trim()
}

if (-not $Ref) {
    throw "Impossible de determiner la branche."
}

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    throw "GitHub CLI 'gh' introuvable."
}

Write-Host "Dispatch BrowserHost sur $Ref" -ForegroundColor Cyan
gh workflow run ladybird-browser-host.yml --ref $Ref
if ($LASTEXITCODE -ne 0) {
    throw "gh workflow run a echoue."
}

Start-Sleep -Seconds 2

$run = (& gh run list `
    --workflow ladybird-browser-host.yml `
    --branch $Ref `
    --limit 1 `
    --json databaseId `
    --jq '.[0].databaseId').Trim()

if (-not $run) {
    throw "Run BrowserHost introuvable."
}

Write-Host "Run BrowserHost : $run" -ForegroundColor Green
Write-Host "Suivi : gh run watch $run" -ForegroundColor DarkGray

if ($Watch) {
    gh run watch $run
}
