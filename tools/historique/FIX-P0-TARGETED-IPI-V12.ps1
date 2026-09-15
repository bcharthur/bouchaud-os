param(
    [switch]$SkipCargoCheck
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ThreadPath = "src\kernel\process\thread.rs"
$IdtPath    = "src\arch\x86_64\idt.rs"
$Marker     = "BOUCHAUD_P0_TARGETED_SCHED_IPI_V1"

function Fail([string]$Message) {
    throw "[P0 v1.2] $Message"
}

function Test-ByteSequence([byte[]]$Data, [byte[]]$Needle) {
    if ($Needle.Length -eq 0 -or $Data.Length -lt $Needle.Length) {
        return $false
    }

    for ($i = 0; $i -le $Data.Length - $Needle.Length; $i++) {
        $ok = $true
        for ($j = 0; $j -lt $Needle.Length; $j++) {
            if ($Data[$i + $j] -ne $Needle[$j]) {
                $ok = $false
                break
            }
        }
        if ($ok) {
            return $true
        }
    }
    return $false
}

function Read-Utf8Strict([string]$Path) {
    $full = (Resolve-Path -LiteralPath $Path).Path
    $bytes = [System.IO.File]::ReadAllBytes($full)
    $utf8 = New-Object System.Text.UTF8Encoding($false, $true)
    return $utf8.GetString($bytes)
}

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    $full = (Resolve-Path -LiteralPath $Path).Path
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($full, $Content, $utf8)
}

if (-not (Test-Path ".git")) {
    Fail "Run this script from the bouchaud-os repository root."
}
if (-not (Test-Path -LiteralPath $ThreadPath)) {
    Fail "Missing file: $ThreadPath"
}
if (-not (Test-Path -LiteralPath $IdtPath)) {
    Fail "Missing file: $IdtPath"
}

Write-Host "=== Bouchaud OS P0 #1 v1.2 ===" -ForegroundColor Cyan
Write-Host "Repair source encoding, then reapply targeted scheduler IPI."
Write-Host ""

# ---------------------------------------------------------------------------
# Locate a known-good pre-v1 backup.
#
# We do not trust file names alone. We validate the exact UTF-8 byte sequence
# of the original identifier:
#
#     n + U+0153 + uds
#
# UTF-8 bytes: 6E C5 93 75 64 73
#
# This keeps this PowerShell file itself strictly ASCII.
# ---------------------------------------------------------------------------

$ExpectedUtf8Token = [byte[]](0x6E, 0xC5, 0x93, 0x75, 0x64, 0x73)

$candidates = Get-ChildItem ".bouchaud-history\backups" -Directory -Force -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like ".bouchaud-p0-targeted-sched-ipi-*" } |
    Sort-Object Name -Descending

$backup = $null

foreach ($candidate in $candidates) {
    $candidateThread = Join-Path $candidate.FullName $ThreadPath
    $candidateIdt = Join-Path $candidate.FullName $IdtPath

    if (-not (Test-Path -LiteralPath $candidateThread)) {
        continue
    }
    if (-not (Test-Path -LiteralPath $candidateIdt)) {
        continue
    }

    $bytes = [System.IO.File]::ReadAllBytes($candidateThread)
    if (Test-ByteSequence $bytes $ExpectedUtf8Token) {
        $backup = $candidate
        break
    }
}

if (-not $backup) {
    Fail "No known-good P0 backup with the expected UTF-8 source bytes was found."
}

$backupThread = Join-Path $backup.FullName $ThreadPath
$backupIdt = Join-Path $backup.FullName $IdtPath

Write-Host "[BACKUP] known-good source: $($backup.FullName)" -ForegroundColor Yellow

# ---------------------------------------------------------------------------
# Restore byte-for-byte.
# Parser failures in v1.1 happened before execution, so the v1 backup is still
# the correct recovery point.
# ---------------------------------------------------------------------------

Copy-Item -LiteralPath $backupThread -Destination $ThreadPath -Force
Copy-Item -LiteralPath $backupIdt -Destination $IdtPath -Force

$sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $backupThread).Hash
$restoredHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $ThreadPath).Hash
if ($sourceHash -ne $restoredHash) {
    Fail "thread.rs binary restore verification failed."
}

$sourceHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $backupIdt).Hash
$restoredHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $IdtPath).Hash
if ($sourceHash -ne $restoredHash) {
    Fail "idt.rs binary restore verification failed."
}

Write-Host "[OK] original files restored byte-for-byte" -ForegroundColor Green

# Strict UTF-8 decode after binary restore.
$thread = Read-Utf8Strict $ThreadPath
$idt = Read-Utf8Strict $IdtPath

# ---------------------------------------------------------------------------
# Patch thread.rs.
# ---------------------------------------------------------------------------

$threadPattern = '(?ms)(pub fn in_user_task\(\) -> bool \{\r?\n\s*current_index_raw\(\) != NO_TASK\r?\n\})'

$threadInsert = @'
pub fn in_user_task() -> bool {
    current_index_raw() != NO_TASK
}

// BOUCHAUD_P0_TARGETED_SCHED_IPI_V1
/// Lock-free mask of secondary CPUs currently running a user task.
///
/// The BSP timer reads only atomics here, before any BKL attempt.
/// Idle CPUs and kernel threads are excluded. A user task temporarily inside a
/// syscall remains included so it can still receive its periodic quantum.
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
$count = $threadRegex.Matches($thread).Count
if ($count -ne 1) {
    Fail "Unexpected thread.rs anchor count: $count"
}

$newThread = $threadRegex.Replace(
    $thread,
    [System.Text.RegularExpressions.MatchEvaluator]{ param($m) $threadInsert },
    1
)

# ---------------------------------------------------------------------------
# Patch idt.rs.
# ---------------------------------------------------------------------------

$idtPattern = '(?ms)\s*let quantum = timer::ticks\(\) % smp::SCHED_QUANTUM_TICKS == 0;\r?\n\s*if quantum && !smp::local_scheduler_timer_enabled\(\) \{\r?\n\s*smp::broadcast_reschedule\(\);\r?\n\s*\}'

$idtReplacement = @'
    let quantum = timer::ticks() % smp::SCHED_QUANTUM_TICKS == 0;
    if quantum && !smp::local_scheduler_timer_enabled() {
        // BOUCHAUD_P0_TARGETED_SCHED_IPI_V1
        //
        // PIT fallback used when local TSC-deadline scheduling is unavailable.
        // Do not broadcast every 4 ms to idle APs. Wake only secondary CPUs
        // that are currently executing a user task.
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
$count = $idtRegex.Matches($idt).Count
if ($count -ne 1) {
    Fail "Unexpected idt.rs anchor count: $count"
}

$newIdt = $idtRegex.Replace(
    $idt,
    [System.Text.RegularExpressions.MatchEvaluator]{ param($m) $idtReplacement },
    1
)

Write-Utf8NoBom $ThreadPath $newThread
Write-Utf8NoBom $IdtPath $newIdt

# ---------------------------------------------------------------------------
# Post-write encoding verification.
# ---------------------------------------------------------------------------

$threadBytes = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $ThreadPath).Path)
if (-not (Test-ByteSequence $threadBytes $ExpectedUtf8Token)) {
    Fail "Original UTF-8 token was not preserved after patching."
}

$threadCheck = Read-Utf8Strict $ThreadPath
$idtCheck = Read-Utf8Strict $IdtPath

if (-not $threadCheck.Contains($Marker)) {
    Fail "P0 marker missing from thread.rs"
}
if (-not $idtCheck.Contains($Marker)) {
    Fail "P0 marker missing from idt.rs"
}
if (-not $threadCheck.Contains("pub fn running_user_cpu_mask() -> u64")) {
    Fail "running_user_cpu_mask() missing"
}

$timerStart = $idtCheck.IndexOf('extern "x86-interrupt" fn timer_interrupt_handler')
$reschedStart = $idtCheck.IndexOf('extern "x86-interrupt" fn reschedule_interrupt_handler', $timerStart)
if ($timerStart -lt 0 -or $reschedStart -lt 0) {
    Fail "Could not locate timer/reschedule handlers."
}

$timerBody = $idtCheck.Substring($timerStart, $reschedStart - $timerStart)
if ($timerBody.Contains("smp::broadcast_reschedule();")) {
    Fail "Periodic scheduler broadcast is still present in timer handler."
}

Write-Host "[OK] UTF-8 source preserved" -ForegroundColor Green
Write-Host "[OK] targeted scheduler IPI patch applied" -ForegroundColor Green

git diff --check
if ($LASTEXITCODE -ne 0) {
    Fail "git diff --check failed."
}

if (-not $SkipCargoCheck) {
    Write-Host ""
    Write-Host "[BUILD] cargo check" -ForegroundColor Cyan
    cargo check
    if ($LASTEXITCODE -ne 0) {
        Fail "cargo check failed."
    }
}

Write-Host ""
Write-Host "P0 v1.2 repair completed." -ForegroundColor Green
Write-Host "Next:" -ForegroundColor Cyan
Write-Host "  .\VERIFY-P0-TARGETED-IPI-V12.ps1 -Build"
Write-Host "  .\run.ps1"
