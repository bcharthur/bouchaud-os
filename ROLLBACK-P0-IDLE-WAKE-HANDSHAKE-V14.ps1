param(
    [string]$Backup = ""
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Backup)) {
    $candidate = Get-ChildItem ".bouchaud-history\backups" -Directory -Force -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like ".bouchaud-p0-idle-wake-handshake-v14-*" } |
        Sort-Object Name -Descending |
        Select-Object -First 1

    if (-not $candidate) {
        throw "No P0 v1.4 backup found."
    }
    $Backup = $candidate.FullName
}

$files = @(
    "src\arch\x86_64\cpu.rs",
    "src\kernel\process\thread.rs"
)

foreach ($file in $files) {
    $source = Join-Path $Backup $file
    if (-not (Test-Path -LiteralPath $source)) {
        throw "Backup file missing: $source"
    }
    Copy-Item -LiteralPath $source -Destination $file -Force
    Write-Host "[RESTORE] $file" -ForegroundColor Yellow
}

Write-Host "Rollback v1.4 completed." -ForegroundColor Green
