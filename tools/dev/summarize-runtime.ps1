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

# BOUCHAUD_GATE1B_MESURE_V1
#
# Les compteurs GUI sont CUMULATIFS depuis le demarrage. Un total ne dit donc
# rien d'une periode d'inactivite : il faut la difference entre deux releves
# consecutifs. C'est cette difference, et elle seule, qui repond a "le bureau
# dort-il quand rien ne bouge".
function Parse-Compteurs([string]$Line) {
    $table = @{}
    if (-not $Line) { return $table }
    foreach ($paire in ([regex]::Matches($Line, "([A-Za-z0-9_]+)=(-?\d+)"))) {
        $table[$paire.Groups[1].Value] = [int64]$paire.Groups[2].Value
    }
    return $table
}

function Deltas([string]$Pattern, [string]$Titre, [string[]]$Cles) {
    $releves = @(Matching $Pattern | ForEach-Object { Parse-Compteurs $_.Line })
    if ($releves.Count -lt 2) {
        Write-Host "$Titre : moins de deux releves, aucun delta calculable" -ForegroundColor DarkYellow
        return
    }
    $avant = $releves[$releves.Count - 2]
    $apres = $releves[$releves.Count - 1]
    Write-Host "$Titre (dernier intervalle de releve)"
    foreach ($cle in $Cles) {
        if ($apres.ContainsKey($cle) -and $avant.ContainsKey($cle)) {
            $d = $apres[$cle] - $avant[$cle]
            Write-Host ("    {0,-26} delta={1,-12} total={2}" -f $cle, $d, $apres[$cle])
        }
    }
}

Write-Host "`n=== GATE 1B / 1C : COMPOSITEUR ===" -ForegroundColor Cyan
foreach ($pattern in @("\[GUI-COMPOSITOR\]", "\[GUI-COMPOSITOR-SOURCES\]", "\[GUI-SCENE\]")) {
    $line = Last-Match $pattern
    if ($line) { Write-Host $line }
}

Deltas "\[GUI-COMPOSITOR\]" "compositeur" @(
    "wakeups", "invalidations", "frames_composed", "frames_clock_only",
    "frames_useful", "frames_skipped", "blind_recomposes",
    "idle_sleeps", "idle_wakeups_signal", "idle_wakeups_deadline", "loops"
)
Deltas "\[GUI-SCENE\]" "culling de scene" @(
    "layers_offered", "layers_drawn", "layers_occluded", "layers_culled"
)
Deltas "\[GUI-DAMAGE\]" "degats" @(
    "presents", "rects", "presented_pixels", "requested_pixels",
    "gate0_bbox_pixels", "saved_pixels", "drawn_pixels", "merges", "overflows"
)

# Le critere d'inactivite. `frames_clock_only` est attendu non nul : l'horloge
# de la barre des taches est la seule animation permanente du bureau, et elle
# impose une trame par seconde. Ce qui doit tomber a zero, c'est le RESTE.
$compos = @(Matching "\[GUI-COMPOSITOR\]" | ForEach-Object { Parse-Compteurs $_.Line })
if ($compos.Count -ge 2) {
    $a = $compos[$compos.Count - 2]
    $b = $compos[$compos.Count - 1]
    if ($b.ContainsKey("frames_useful") -and $a.ContainsKey("frames_useful")) {
        $utiles = $b["frames_useful"] - $a["frames_useful"]
        $horloge = $b["frames_clock_only"] - $a["frames_clock_only"]
        Write-Host ""
        Write-Host ("IDLE frames_useful_delta={0} frames_clock_only_delta={1}" -f $utiles, $horloge)
        if ($utiles -eq 0) {
            Write-Host "IDLE_OK : aucune trame hors horloge sur le dernier intervalle." -ForegroundColor Green
        } else {
            Write-Host "IDLE_ACTIF : $utiles trame(s) utile(s) -- le bureau avait du travail." -ForegroundColor DarkYellow
        }
    }
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
