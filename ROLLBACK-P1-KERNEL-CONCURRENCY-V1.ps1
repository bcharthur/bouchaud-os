param(
    [string]$Backup = ""
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Backup)) {
    $candidate = Get-ChildItem ".bouchaud-history\backups" -Directory -Force -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like ".bouchaud-p1-kernel-concurrency-v1-*" } |
        Sort-Object Name -Descending |
        Select-Object -First 1

    if (-not $candidate) {
        throw "No P1 concurrency backup found."
    }
    $Backup = $candidate.FullName
}

$files = @(
    "src\kernel\sync\bkl.rs",
    "src\kernel\sync\wait_queue.rs",
    "src\kernel\process\thread.rs",
    "src\compat\linux\file.rs",
    "src\compat\linux\bkl.rs",
    "tools\verifie-verrouillage.py"
)

foreach ($file in $files) {
    $source = Join-Path $Backup $file
    if (-not (Test-Path -LiteralPath $source)) {
        throw "Backup file missing: $source"
    }
    Copy-Item -LiteralPath $source -Destination $file -Force
    Write-Host "[RESTORE] $file" -ForegroundColor Yellow
}

Write-Host "P1 rollback completed." -ForegroundColor Green
