param(
    [switch]$Preview
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ThreadPath = "src\kernel\process\thread.rs"
$IdtPath    = "src\arch\x86_64\idt.rs"

function Fail([string]$Message) {
    throw "[P0 targeted IPI v1.1] $Message"
}

if (-not (Test-Path ".git")) {
    Fail "Lance ce script depuis la racine du depot bouchaud-os."
}

# IMPORTANT: Windows PowerShell 5.1 ne doit pas utiliser Get-Content -Raw ici.
# Les sources sont UTF-8 sans BOM. On force explicitement UTF-8.
$Utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)
$Utf8NoBom  = New-Object System.Text.UTF8Encoding($false)

function Read-Utf8Strict([string]$Path) {
    return [System.IO.File]::ReadAllText((Resolve-Path -LiteralPath $Path).Path, $Utf8Strict)
}
function Write-Utf8NoBom([string]$Path, [string]$Content) {
    [System.IO.File]::WriteAllText((Resolve-Path -LiteralPath $Path).Path, $Content, $Utf8NoBom)
}

$thread = Read-Utf8Strict $ThreadPath
$idt    = Read-Utf8Strict $IdtPath
$Marker = "BOUCHAUD_P0_TARGETED_SCHED_IPI_V1"

if ($thread.Contains($Marker) -and $idt.Contains($Marker)) {
    Write-Host "[OK] P0 deja applique." -ForegroundColor Green
    exit 0
}
if ($thread.Contains($Marker) -xor $idt.Contains($Marker)) {
    Fail "Patch partiellement applique."
}

$threadPattern = '(?ms)(pub fn in_user_task\(\) -> bool \{\r?\n\s*current_index_raw\(\) != NO_TASK\r?\n\})'
$threadInsert = @'
pub fn in_user_task() -> bool {
    current_index_raw() != NO_TASK
}

// BOUCHAUD_P0_TARGETED_SCHED_IPI_V1
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

$idtPattern = '(?ms)\s*let quantum = timer::ticks\(\) % smp::SCHED_QUANTUM_TICKS == 0;\r?\n\s*if quantum && !smp::local_scheduler_timer_enabled\(\) \{\r?\n\s*smp::broadcast_reschedule\(\);\r?\n\s*\}'
$idtReplacement = @'
    let quantum = timer::ticks() % smp::SCHED_QUANTUM_TICKS == 0;
    if quantum && !smp::local_scheduler_timer_enabled() {
        // BOUCHAUD_P0_TARGETED_SCHED_IPI_V1
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

$tr = [regex]::new($threadPattern)
$ir = [regex]::new($idtPattern)

if ($tr.Matches($thread).Count -ne 1) { Fail "Ancre thread.rs inattendue." }
if ($ir.Matches($idt).Count -ne 1) { Fail "Ancre idt.rs inattendue." }

if ($Preview) {
    Write-Host "[PREVIEW] patch UTF-8-safe applicable." -ForegroundColor Yellow
    exit 0
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$backupRoot = ".bouchaud-history\backups\.bouchaud-p0-targeted-sched-ipi-v11-$stamp"
New-Item -ItemType Directory -Force "$backupRoot\src\kernel\process" | Out-Null
New-Item -ItemType Directory -Force "$backupRoot\src\arch\x86_64" | Out-Null
Copy-Item -LiteralPath $ThreadPath -Destination "$backupRoot\$ThreadPath" -Force
Copy-Item -LiteralPath $IdtPath -Destination "$backupRoot\$IdtPath" -Force

$newThread = $tr.Replace($thread, [System.Text.RegularExpressions.MatchEvaluator]{param($m) $threadInsert}, 1)
$newIdt = $ir.Replace($idt, [System.Text.RegularExpressions.MatchEvaluator]{param($m) $idtReplacement}, 1)

Write-Utf8NoBom $ThreadPath $newThread
Write-Utf8NoBom $IdtPath $newIdt

Write-Host "[OK] P0 applique sans conversion ANSI." -ForegroundColor Green
