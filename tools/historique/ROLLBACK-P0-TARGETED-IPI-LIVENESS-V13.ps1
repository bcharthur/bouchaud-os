param(
    [string]$Backup = ""
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Backup)) {
    $candidate = Get-ChildItem ".bouchaud-history\backups" -Directory -Force -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like ".bouchaud-p0-targeted-ipi-liveness-v13-*" } |
        Sort-Object Name -Descending |
        Select-Object -First 1

    if (-not $candidate) {
        throw "No P0 v1.3 backup found."
    }
    $Backup = $candidate.FullName
}

$source = Join-Path $Backup "src\kernel\sync\bkl.rs"
if (-not (Test-Path -LiteralPath $source)) {
    throw "Backup file missing: $source"
}

Copy-Item -LiteralPath $source -Destination "src\kernel\sync\bkl.rs" -Force
Write-Host "[RESTORE] src\kernel\sync\bkl.rs" -ForegroundColor Yellow
Write-Host "Rollback v1.3 completed." -ForegroundColor Green
