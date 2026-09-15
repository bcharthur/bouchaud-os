param(
    [Parameter(Mandatory=$true)]
    [string]$Branch
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path ".git")) {
    throw "Run this inside the cloned bouchaud-os repository."
}

git fetch origin
if ($LASTEXITCODE -ne 0) { throw "git fetch failed" }

git switch $Branch
if ($LASTEXITCODE -ne 0) {
    git switch --track "origin/$Branch"
    if ($LASTEXITCODE -ne 0) { throw "Could not switch to $Branch" }
}

git pull --ff-only
if ($LASTEXITCODE -ne 0) { throw "git pull --ff-only failed" }

cargo check
if ($LASTEXITCODE -ne 0) { throw "cargo check failed" }

Write-Host ""
Write-Host "Checkpoint ready." -ForegroundColor Green
Write-Host "Preview P1 with:" -ForegroundColor Cyan
Write-Host "  .\APPLY-P1-KERNEL-CONCURRENCY-V1.ps1 -Preview"
