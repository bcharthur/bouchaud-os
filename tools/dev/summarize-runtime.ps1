param(
    [Parameter(Mandatory=$true, Position=0)]
    [string]$LogPath
)

$ErrorActionPreference = "Stop"
$resolved = (Resolve-Path $LogPath).Path
$lines = Get-Content -Path $resolved

function Matching([string]$Pattern) {
    @($lines | Select-String -Pattern $Pattern)
}

function Last-Match([string]$Pattern) {
    $m = Matching $Pattern
    if ($m.Count -eq 0) { return $null }
    return $m[-1].Line.Trim()
}

function Count-Match([string]$Pattern) {
    return (Matching $Pattern).Count
}

$markers = [ordered]@{
    BROWSER_HOST_INITIALIZED = (Count-Match "BROWSER_HOST_INITIALIZED")
    WEBCONTENT_READY = (Count-Match "WEBCONTENT_READY")
    M11_READY = (Count-Match "M11_READY")
    M11_GUI_HANDSHAKE_OK = (Count-Match "M11_GUI_HANDSHAKE_OK")
    M11_DOCUMENT_LOADED = (Count-Match "M11_DOCUMENT_LOADED")
}

$fatalPatterns = [ordered]@{
    KERNEL_PANIC = "KERNEL PANIC"
    DOUBLE_FAULT = "DOUBLE FAULT"
    PAGE_FAULT_FATAL = "page fault.*fatal|fatal.*page fault"
    GP_FAULT = "general protection|GENERAL PROTECTION"
    BKL_VIOLATION = "smp_lock: release|smp_lock: suspend|smp_lock:.*proprietaire"
}

$fatalCount = 0
$fatalRows = foreach ($kv in $fatalPatterns.GetEnumerator()) {
    $n = Count-Match $kv.Value
    $fatalCount += $n
    [pscustomobject]@{ Signal = $kv.Key; Count = $n }
}

Write-Host "Bouchaud OS - resume runtime" -ForegroundColor Green
Write-Host "log : $resolved"

Write-Host "`n=== GATE LADYBIRD ===" -ForegroundColor Cyan
$markerRows = foreach ($kv in $markers.GetEnumerator()) {
    [pscustomobject]@{
        Marker = $kv.Key
        Count = $kv.Value
        Status = if ($kv.Value -gt 0) { "OK" } else { "MISSING" }
    }
}
$markerRows | Format-Table -AutoSize

Write-Host "`n=== FATAL ===" -ForegroundColor Cyan
$fatalRows | Format-Table -AutoSize

Write-Host "`n=== DERNIERS COMPTEURS ===" -ForegroundColor Cyan
foreach ($pattern in @(
    "\[GUI-DAMAGE\]",
    "\[GUI-INPUT\]",
    "\[BKL-MAX-HOLD\]",
    "\[BKL-STATS\]",
    "\[SMP-SNAPSHOT\]",
    "\[SMP-STALL\]"
)) {
    $line = Last-Match $pattern
    if ($line) { Write-Host $line }
}

$bklFr = Matching "\[BKL-FR\]"
if ($bklFr.Count -gt 0) {
    Write-Host "`n=== BKL FLIGHT RECORDER (dernieres 64 lignes) ===" -ForegroundColor Yellow
    @($bklFr | Select-Object -Last 64) | ForEach-Object { Write-Host $_.Line }
}

$faults = Matching "\[FAULT\]|DOUBLE FAULT|KERNEL PANIC|smp_lock:"
if ($faults.Count -gt 0) {
    Write-Host "`n=== CONTEXTE DE FAUTE (dernieres 80 lignes pertinentes) ===" -ForegroundColor Yellow
    @($faults | Select-Object -Last 80) | ForEach-Object { Write-Host $_.Line }
}

$allMarkers = ($markers.Values | Where-Object { $_ -le 0 }).Count -eq 0
$pass = $allMarkers -and ($fatalCount -eq 0)

Write-Host "`n=== VERDICT ===" -ForegroundColor Cyan
if ($pass) {
    Write-Host "GATE0_RUNTIME=PASS" -ForegroundColor Green
    Write-Host "Ladybird a atteint tous les marqueurs sans signal fatal detecte."
    exit 0
} else {
    Write-Host "GATE0_RUNTIME=FAIL" -ForegroundColor Red
    if (-not $allMarkers) { Write-Host "Cause: marqueur(s) Ladybird manquant(s)." }
    if ($fatalCount -gt 0) { Write-Host "Cause: $fatalCount signal(aux) fatal(aux) detecte(s)." }
    exit 1
}
