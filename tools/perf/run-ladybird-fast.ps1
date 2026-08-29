param(
    [string]$Url = "https://www.google.com/",
    [ValidateRange(2048,16384)][int]$RamMiB = 12288,
    [switch]$TcgSmp
)
$ErrorActionPreference = "Stop"
$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
Set-Location $Root

if ($TcgSmp) {
    Write-Host "V13 FAST: TCG SMP 4 vCPU (validation SMP, plus lent)" -ForegroundColor Yellow
    & .\run.ps1 -Ladybird -LadybirdUrl $Url -RamMiB $RamMiB -CpuCount 4 -Accel tcg
    exit $LASTEXITCODE
}

Write-Host "V13 FAST: WHPX + 1 vCPU (mesure navigateur / UX)" -ForegroundColor Green
Write-Host "Le chemin WHPX SMP 4 expose encore un probleme APIC; on ne le confond pas avec les performances du navigateur." -ForegroundColor Yellow
& .\run.ps1 -Ladybird -LadybirdUrl $Url -RamMiB $RamMiB -CpuCount 1 -Accel whpx
exit $LASTEXITCODE
