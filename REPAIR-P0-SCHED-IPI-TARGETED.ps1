param(
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ThreadPath = "src\kernel\process\thread.rs"
$IdtPath    = "src\arch\x86_64\idt.rs"
$Marker     = "BOUCHAUD_P0_TARGETED_SCHED_IPI_V1"

function Fail([string]$Message) {
    throw "[P0 v1.1 hotfix] $Message"
}

if (-not (Test-Path ".git")) {
    Fail "Lance ce script depuis la racine du depot bouchaud-os."
}

# Windows PowerShell 5.1 peut decoder un UTF-8 sans BOM comme ANSI avec
# Get-Content -Raw. On utilise donc explicitement un decodeur UTF-8 strict.
$Utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)
$Utf8NoBom  = New-Object System.Text.UTF8Encoding($false)

function Read-Utf8Strict([string]$Path) {
    $full = (Resolve-Path -LiteralPath $Path).Path
    return [System.IO.File]::ReadAllText($full, $Utf8Strict)
}

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    $full = (Resolve-Path -LiteralPath $Path).Path
    [System.IO.File]::WriteAllText($full, $Content, $Utf8NoBom)
}

Write-Host "=== Bouchaud OS P0 #1 v1.1 : repair UTF-8 + targeted IPI ===" -ForegroundColor Cyan

# Le script v1 a heureusement fait un Copy-Item binaire AVANT la conversion.
# On restaure donc les octets exacts du dernier backup P0.
$backup = Get-ChildItem ".bouchaud-history\backups" -Directory -Force -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like ".bouchaud-p0-targeted-sched-ipi-*" } |
    Sort-Object Name -Descending |
    Select-Object -First 1

if (-not $backup) {
    Fail "Aucun backup .bouchaud-p0-targeted-sched-ipi-* trouve."
}

$backupThread = Join-Path $backup.FullName $ThreadPath
$backupIdt    = Join-Path $backup.FullName $IdtPath

if (-not (Test-Path -LiteralPath $backupThread)) {
    Fail "Backup thread.rs absent: $backupThread"
}
if (-not (Test-Path -LiteralPath $backupIdt)) {
    Fail "Backup idt.rs absent: $backupIdt"
}

Write-Host "[RESTORE] $ThreadPath <- $($backup.Name)" -ForegroundColor Yellow
Write-Host "[RESTORE] $IdtPath    <- $($backup.Name)" -ForegroundColor Yellow
Copy-Item -LiteralPath $backupThread -Destination $ThreadPath -Force
Copy-Item -LiteralPath $backupIdt -Destination $IdtPath -Force

# Validation de l'encodage restaure.
$thread = Read-Utf8Strict $ThreadPath
$idt    = Read-Utf8Strict $IdtPath

if (-not $thread.Contains("let nœuds:")) {
    Fail "Le backup restaure ne contient pas le token UTF-8 attendu 'nœuds'. Arret sans reappliquer."
}
if ($thread.Contains("nÅ") -or $thread.Contains("â€")) {
    Fail "Mojibake encore present apres restauration. Arret."
}
Write-Host "[OK] UTF-8 source restaure (nœuds lisible)" -ForegroundColor Green

# ---------------------------------------------------------------------------
# Reapplication du patch, mais avec lecture/ecriture UTF-8 explicite.
# ---------------------------------------------------------------------------

if ($thread.Contains($Marker) -or $idt.Contains($Marker)) {
    Fail "Le backup contient deja le marqueur P0, ce qui est inattendu."
}

$threadPattern = '(?ms)(pub fn in_user_task\(\) -> bool \{\r?\n\s*current_index_raw\(\) != NO_TASK\r?\n\})'
$threadInsert = @'
pub fn in_user_task() -> bool {
    current_index_raw() != NO_TASK
}

// BOUCHAUD_P0_TARGETED_SCHED_IPI_V1
/// Masque lock-free des CPU secondaires qui executent actuellement une tache
/// utilisateur.
///
/// Cette lecture est volontairement composee uniquement d'atomiques : elle est
/// appelee depuis l'IRQ timer du BSP avant toute tentative de prise du BKL.
///
/// Un CPU idle est exclu. Un fil noyau est exclu. Une tache utilisateur entree
/// momentanement dans un syscall reste incluse : elle doit continuer a recevoir
/// son quantum et pourra poser une preemption differee.
///
/// Une course avec une transition idle<->user est sans danger :
/// - un reveil de nouvelle tache envoie deja un IPI cible via `smp::reschedule_cpu`;
/// - au pire un CPU recoit un IPI devenu inutile, ou attend le quantum suivant.
pub fn running_user_cpu_mask() -> u64 {
    let online = smp::schedulable_cpus().min(MAX_CPUS).min(64);
    let mut mask = 0u64;
    let mut cpu = 1usize;

    while cpu < online {
        let current = CURRENT[cpu].load(Ordering::Acquire);
        let kernel_task = CURRENT_IS_KERNEL[cpu].load(Ordering::Acquire);
        if current != NO_TASK && !kernel_task {
            mask |= 1u64 << cpu;
        }
        cpu += 1;
    }

    mask
}
'@

$threadRegex = [regex]::new($threadPattern)
if ($threadRegex.Matches($thread).Count -ne 1) {
    Fail "Ancre in_user_task() inattendue. Aucun patch applique."
}
$newThread = $threadRegex.Replace(
    $thread,
    [System.Text.RegularExpressions.MatchEvaluator]{ param($m) $threadInsert },
    1
)

$idtPattern = '(?ms)\s*let quantum = timer::ticks\(\) % smp::SCHED_QUANTUM_TICKS == 0;\r?\n\s*if quantum && !smp::local_scheduler_timer_enabled\(\) \{\r?\n\s*smp::broadcast_reschedule\(\);\r?\n\s*\}'
$idtReplacement = @'
    let quantum = timer::ticks() % smp::SCHED_QUANTUM_TICKS == 0;
    if quantum && !smp::local_scheduler_timer_enabled() {
        // BOUCHAUD_P0_TARGETED_SCHED_IPI_V1
        //
        // Fallback PIT quand TSC-deadline n'est pas disponible (notamment TCG).
        // Le BSP ne reveille plus tous les AP a chaque quantum : seuls les CPU
        // qui executent une tache utilisateur recoivent l'IPI periodique.
        let targets = crate::kernel::task::running_user_cpu_mask();
        let online = smp::schedulable_cpus().min(64);
        let mut cpu = 1usize;
        while cpu < online {
            if targets & (1u64 << cpu) != 0 {
                smp::reschedule_cpu(cpu);
            }
            cpu += 1;
        }
    }
'@

$idtRegex = [regex]::new($idtPattern)
if ($idtRegex.Matches($idt).Count -ne 1) {
    Fail "Ancre broadcast_reschedule() inattendue. Aucun patch applique."
}
$newIdt = $idtRegex.Replace(
    $idt,
    [System.Text.RegularExpressions.MatchEvaluator]{ param($m) $idtReplacement },
    1
)

Write-Utf8NoBom $ThreadPath $newThread
Write-Utf8NoBom $IdtPath $newIdt

# Relecture stricte apres ecriture.
$checkThread = Read-Utf8Strict $ThreadPath
$checkIdt = Read-Utf8Strict $IdtPath

if (-not $checkThread.Contains("let nœuds:")) {
    Fail "Regression UTF-8 apres patch."
}
if ($checkThread.Contains("nÅ") -or $checkThread.Contains("â€")) {
    Fail "Mojibake detecte apres patch."
}
if (-not $checkThread.Contains($Marker) -or -not $checkIdt.Contains($Marker)) {
    Fail "Marqueur P0 absent apres patch."
}

Write-Host "[OK] UTF-8 preserve" -ForegroundColor Green
Write-Host "[OK] targeted IPI reapplique" -ForegroundColor Green

git diff --check
if ($LASTEXITCODE -ne 0) {
    Fail "git diff --check a echoue."
}

if (-not $SkipBuild) {
    Write-Host ""
    Write-Host "[BUILD] cargo check" -ForegroundColor Cyan
    cargo check
    if ($LASTEXITCODE -ne 0) {
        Fail "cargo check a echoue."
    }
}

Write-Host ""
Write-Host "HOTFIX TERMINE." -ForegroundColor Green
Write-Host "Tu peux maintenant lancer :" -ForegroundColor Cyan
Write-Host "  .\VERIFY-P0-SCHED-IPI-TARGETED.ps1 -Build"
Write-Host "  .\run.ps1"
