param(
    [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
)

$run = Join-Path $RepoRoot "run.ps1"
$wrapper = Join-Path $RepoRoot "tools\run\cargo-bootimage-safe.cmd"

if (-not (Test-Path $run)) {
    Write-Error "run.ps1 introuvable"
    exit 1
}
if (-not (Test-Path $wrapper)) {
    Write-Error "cargo-bootimage-safe.cmd introuvable"
    exit 2
}

$text = Get-Content -Raw -Path $run
$expected = '& "$RepoRoot\tools\run\cargo-bootimage-safe.cmd"'

if ($text -notlike "*$expected*") {
    Write-Error "run.ps1 n'utilise pas encore le wrapper cargo"
    exit 3
}

if ($text -match '(?m)^\s*cargo bootimage\s*$') {
    Write-Error "ancien appel direct cargo bootimage encore present"
    exit 4
}

Write-Host "V13.3.1 runner contract: OK"
exit 0
