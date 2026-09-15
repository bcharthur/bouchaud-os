param([string]$Url = "https://www.google.com/")
$ErrorActionPreference = "Stop"
if (-not (Test-Path ".\run.ps1")) { throw "Lance ce script depuis la racine du depot." }

$log = ".\webcontent-liveness-$(Get-Date -Format 'yyyyMMdd-HHmmss').log"
Write-Host "Log: $log" -ForegroundColor Cyan
Write-Host "Charge Google, scroll/clics 60-120 s. Si ca semble fige, attends 20 s avant Ctrl-C." -ForegroundColor Yellow

.\run.ps1 `
    -Ladybird `
    -LadybirdUrl $Url `
    -CpuCount 4 `
    -Accel tcg |
    Tee-Object -FilePath $log

.\tools\dev\summarize-runtime.ps1 $log
Write-Host "`n=== RX IRQ / POLL / LIVENESS ===" -ForegroundColor Cyan
Select-String `
    -Path $log `
    -Pattern 'e1000: RX interrupt-driven','e1000: IRQ RX indisponible','\[BKL-SYSCALL\]','WEBCONTENT_READY','M11_DOCUMENT_LOADED','client pid=.*silence','KERNEL PANIC','DOUBLE FAULT' |
    Select-Object -Last 100
Write-Host "`nLog a conserver: $log" -ForegroundColor Green
