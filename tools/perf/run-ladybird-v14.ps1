param(
    [string]$Url = "https://www.google.com/",
    [ValidateRange(2048, 16384)][int]$RamMiB = 12288,
    [switch]$TcgSmp,
    [switch]$TcgSmp6,
    [switch]$WhpxSmpExperimental,
    [switch]$Audio
)

$ErrorActionPreference = "Stop"
$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Run = Join-Path $RepoRoot "run.ps1"
if (-not (Test-Path $Run)) { throw "run.ps1 introuvable: $Run" }

if ($TcgSmp6) {
    $CpuCount = 6
    $Accel = "tcg"
    $Profile = "TCG SMP6 experimental throughput"
}
elseif ($TcgSmp) {
    $CpuCount = 4
    $Accel = "tcg"
    $Profile = "TCG SMP4 validation"
}
elseif ($WhpxSmpExperimental) {
    $CpuCount = 4
    $Accel = "whpx"
    $Profile = "WHPX SMP4 experimental (APIC not yet proven)"
}
else {
    # V14 production/UX profile. Four emulated TCG vCPUs are not four native
    # cores; for interactive browsing, one hardware-accelerated CPU is often
    # dramatically faster. SMP correctness remains tested with -TcgSmp.
    $CpuCount = 1
    $Accel = "auto"
    $Profile = "WHPX1 interactive (fallback TCG1)"
}

$AudioBackend = if ($Audio) { "dsound" } else { "none" }
Write-Host "V14 profile : $Profile" -ForegroundColor Cyan
Write-Host "RAM         : $RamMiB MiB" -ForegroundColor DarkGray
Write-Host "Audio       : $AudioBackend" -ForegroundColor DarkGray

& $Run -Ladybird -LadybirdUrl $Url -RamMiB $RamMiB -CpuCount $CpuCount -Accel $Accel -Audio $AudioBackend
exit $LASTEXITCODE
