param(
    [string]$Backup = ""
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Backup)) {
    $candidate = Get-ChildItem ".bouchaud-history\backups" -Directory -Force -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like ".bouchaud-p0-targeted-sched-ipi-*" } |
        Sort-Object Name -Descending |
        Select-Object -First 1

    if (-not $candidate) {
        throw "Aucun backup P0 trouve."
    }
    $Backup = $candidate.FullName
}

$files = @(
    "src\kernel\process\thread.rs",
    "src\arch\x86_64\idt.rs"
)

foreach ($file in $files) {
    $source = Join-Path $Backup $file
    if (-not (Test-Path -LiteralPath $source)) {
        throw "Backup incomplet: $source"
    }
    Copy-Item -LiteralPath $source -Destination $file -Force
    Write-Host "[RESTORE] $file" -ForegroundColor Yellow
}

Write-Host "Rollback termine." -ForegroundColor Green
