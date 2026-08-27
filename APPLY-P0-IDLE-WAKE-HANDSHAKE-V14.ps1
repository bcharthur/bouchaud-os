param(
    [switch]$Preview,
    [switch]$SkipCargoCheck
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$CpuPath = "src\arch\x86_64\cpu.rs"
$ThreadPath = "src\kernel\process\thread.rs"
$BklPath = "src\kernel\sync\bkl.rs"
$IdtPath = "src\arch\x86_64\idt.rs"

$TargetedMarker = "BOUCHAUD_P0_TARGETED_SCHED_IPI_V1"
$V13Marker = "BOUCHAUD_P0_TARGETED_IPI_LIVENESS_V13"
$V14Marker = "BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14"

function Fail([string]$Message) {
    throw "[P0 v1.4 idle handshake] $Message"
}

if (-not (Test-Path ".git")) {
    Fail "Run this script from the bouchaud-os repository root."
}

foreach ($path in @($CpuPath, $ThreadPath, $BklPath, $IdtPath)) {
    if (-not (Test-Path -LiteralPath $path)) {
        Fail "Missing file: $path"
    }
}

$Utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)
$Utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Read-Utf8Strict([string]$Path) {
    $full = (Resolve-Path -LiteralPath $Path).Path
    $bytes = [System.IO.File]::ReadAllBytes($full)
    return $Utf8Strict.GetString($bytes)
}

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    $full = (Resolve-Path -LiteralPath $Path).Path
    [System.IO.File]::WriteAllText($full, $Content, $Utf8NoBom)
}

$cpu = Read-Utf8Strict $CpuPath
$thread = Read-Utf8Strict $ThreadPath
$bkl = Read-Utf8Strict $BklPath
$idt = Read-Utf8Strict $IdtPath

if (-not $thread.Contains($TargetedMarker) -or -not $idt.Contains($TargetedMarker)) {
    Fail "Targeted-IPI P0 is not applied."
}
if (-not $bkl.Contains($V13Marker)) {
    Fail "P0 v1.3 liveness fix is not applied. Apply v1.3 first."
}

if ($cpu.Contains($V14Marker) -and $thread.Contains($V14Marker)) {
    Write-Host "[OK] P0 v1.4 already applied." -ForegroundColor Green
    exit 0
}

# ---------------------------------------------------------------------------
# 1) CPU primitive: publish IDLE with IF=0, then use the canonical STI;HLT
#    interrupt-shadow sequence. This primitive is deliberately separate from
#    wait_for_interrupt(), because only scheduler sleeps have the BKL handshake.
# ---------------------------------------------------------------------------

$cpuAnchor = @'
pub fn wait_for_interrupt() {
    let cpu = hardware_cpu_index();
    idle_enter(cpu);
    unsafe { asm!("sti; hlt", options(nostack)); }
    idle_exit(cpu);
}
'@

if (-not $cpu.Contains($cpuAnchor)) {
    $cpuAnchor = $cpuAnchor.Replace("`r`n", "`n")
}
if (-not $cpu.Contains($cpuAnchor)) {
    Fail "cpu::wait_for_interrupt() anchor not found."
}

$cpuReplacement = @'
pub fn wait_for_interrupt() {
    let cpu = hardware_cpu_index();
    idle_enter(cpu);
    unsafe { asm!("sti; hlt", options(nostack)); }
    idle_exit(cpu);
}

// BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14
//
// Scheduler sleep is a two-phase handshake.
//
// PREPARE runs while the caller still owns the BKL:
//   1. IF is cleared;
//   2. IDLE[cpu] becomes visible.
//
// Only then may the caller release the BKL. Every normal Ready publication is
// serialized by that same BKL, so a producer can no longer enqueue work in the
// old "not idle yet / already released the BKL" window.
//
// COMMIT uses the architectural STI;HLT interrupt shadow. If a targeted wakeup
// became pending after PREPARE, HLT executes atomically with respect to that
// pending interrupt and returns immediately instead of losing the wakeup.
pub fn prepare_scheduler_idle() {
    debug_assert!(
        interrupts_enabled(),
        "cpu: prepare_scheduler_idle requires IF=1"
    );
    let cpu = hardware_cpu_index();
    unsafe { asm!("cli", options(nomem, nostack)); }
    idle_enter(cpu);
}

pub fn commit_scheduler_idle() {
    debug_assert!(
        !interrupts_enabled(),
        "cpu: commit_scheduler_idle requires IF=0"
    );
    let cpu = hardware_cpu_index();
    unsafe { asm!("sti; hlt", options(nostack)); }
    idle_exit(cpu);
}
'@

$newCpu = $cpu.Replace($cpuAnchor, $cpuReplacement)

# ---------------------------------------------------------------------------
# 2) Scheduler-side handshake.
#
# Every scheduler HLT must:
#   prepare_scheduler_idle();  // while BKL is still owned
#   suspend_for_schedule();    // releases BKL with IF already 0
#   commit_scheduler_idle();   // STI;HLT, then IDLE=false after wake
#   resume_after_schedule();
# ---------------------------------------------------------------------------

$newThread = $thread

# Direct ABI wait.
$old = @'
pub fn wait_for_interrupt_releasing_bkl() {
    debug_assert_interrupts_enabled();
    let depth = smp_lock::suspend_for_schedule();

    #[cfg(debug_assertions)]
    debug_assert!(
        !smp_lock::held_by_current_cpu(),
        "task: HLT interdit tant que le BKL est detenu"
    );

    cpu::wait_for_interrupt();
    smp_lock::resume_after_schedule(depth);
}
'@
if (-not $newThread.Contains($old)) { $old = $old.Replace("`r`n", "`n") }
if (-not $newThread.Contains($old)) { Fail "ABI wait block not found." }

$new = @'
pub fn wait_for_interrupt_releasing_bkl() {
    debug_assert_interrupts_enabled();
    // BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14
    cpu::prepare_scheduler_idle();
    let depth = smp_lock::suspend_for_schedule();

    #[cfg(debug_assertions)]
    debug_assert!(
        !smp_lock::held_by_current_cpu(),
        "task: HLT interdit tant que le BKL est detenu"
    );

    cpu::commit_scheduler_idle();
    smp_lock::resume_after_schedule(depth);
}
'@
$newThread = $newThread.Replace($old, $new)

# schedule(): blocked current task, no runnable replacement.
$old = @'
                // Ne jamais dormir en tenant le BKL : les autres CPU doivent
                // pouvoir entrer dans leurs syscalls pendant notre HLT.
                let depth = smp_lock::suspend_for_schedule();
                #[cfg(debug_assertions)]
                debug_assert!(
                    !smp_lock::held_by_current_cpu(),
                    "task: schedule HLT interdit tant que le BKL est detenu"
                );
                cpu::wait_for_interrupt();
                smp_lock::resume_after_schedule(depth);
'@
if (-not $newThread.Contains($old)) { $old = $old.Replace("`r`n", "`n") }
if (-not $newThread.Contains($old)) { Fail "schedule() idle block not found." }

$new = @'
                // Ne jamais dormir en tenant le BKL : les autres CPU doivent
                // pouvoir entrer dans leurs syscalls pendant notre HLT.
                // BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14
                cpu::prepare_scheduler_idle();
                let depth = smp_lock::suspend_for_schedule();
                #[cfg(debug_assertions)]
                debug_assert!(
                    !smp_lock::held_by_current_cpu(),
                    "task: schedule HLT interdit tant que le BKL est detenu"
                );
                cpu::commit_scheduler_idle();
                smp_lock::resume_after_schedule(depth);
'@
$newThread = $newThread.Replace($old, $new)

# secondary_cpu_loop(): before the first task exists.
$old = @'
        if aucune_tache {
            let depth = smp_lock::suspend_for_schedule();
            stall_site_clear();
            cpu::wait_for_interrupt();
            stall_site_set(52, current_index_raw() as u64);
            smp_lock::resume_after_schedule(depth);
            stall_site_set(53, current_index_raw() as u64);
            continue;
        }
'@
if (-not $newThread.Contains($old)) { $old = $old.Replace("`r`n", "`n") }
if (-not $newThread.Contains($old)) { Fail "secondary_cpu_loop() initial idle block not found." }

$new = @'
        if aucune_tache {
            // BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14
            cpu::prepare_scheduler_idle();
            let depth = smp_lock::suspend_for_schedule();
            stall_site_clear();
            cpu::commit_scheduler_idle();
            stall_site_set(52, current_index_raw() as u64);
            smp_lock::resume_after_schedule(depth);
            stall_site_set(53, current_index_raw() as u64);
            continue;
        }
'@
$newThread = $newThread.Replace($old, $new)

# secondary_cpu_loop(): normal no-work path.
$old = @'
        } else {
            let depth = smp_lock::suspend_for_schedule();
            stall_site_clear();
            cpu::wait_for_interrupt();
            stall_site_set(52, current_index_raw() as u64);
            smp_lock::resume_after_schedule(depth);
            stall_site_set(53, current_index_raw() as u64);
        }
'@
if (-not $newThread.Contains($old)) { $old = $old.Replace("`r`n", "`n") }
if (-not $newThread.Contains($old)) { Fail "secondary_cpu_loop() normal idle block not found." }

$new = @'
        } else {
            // BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14
            cpu::prepare_scheduler_idle();
            let depth = smp_lock::suspend_for_schedule();
            stall_site_clear();
            cpu::commit_scheduler_idle();
            stall_site_set(52, current_index_raw() as u64);
            smp_lock::resume_after_schedule(depth);
            stall_site_set(53, current_index_raw() as u64);
        }
'@
$newThread = $newThread.Replace($old, $new)

# BSP idle loop in exit_current().
$old = @'
        let depth = smp_lock::suspend_for_schedule();
        cpu::wait_for_interrupt();
        smp_lock::resume_after_schedule(depth);
        if tasks().iter().any(|t| runnable_local(t, 0) || runnable_steal(t, 0)) {
'@
if (-not $newThread.Contains($old)) { $old = $old.Replace("`r`n", "`n") }
if (-not $newThread.Contains($old)) { Fail "BSP exit_current() idle block not found." }

$new = @'
        // BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14
        cpu::prepare_scheduler_idle();
        let depth = smp_lock::suspend_for_schedule();
        cpu::commit_scheduler_idle();
        smp_lock::resume_after_schedule(depth);
        if tasks().iter().any(|t| runnable_local(t, 0) || runnable_steal(t, 0)) {
'@
$newThread = $newThread.Replace($old, $new)

# Safety checks before write.
if (-not $newCpu.Contains($V14Marker)) {
    Fail "CPU handshake marker was not inserted."
}
if (-not $newThread.Contains($V14Marker)) {
    Fail "Scheduler handshake marker was not inserted."
}

$prepareCount = ([regex]::Matches($newThread, 'cpu::prepare_scheduler_idle\(\);')).Count
$commitCount = ([regex]::Matches($newThread, 'cpu::commit_scheduler_idle\(\);')).Count
if ($prepareCount -ne 5 -or $commitCount -ne 5) {
    Fail "Expected 5 scheduler handshake sites, got prepare=$prepareCount commit=$commitCount."
}

# The old scheduler sleep form must be gone from thread.rs.
$badSleep = [regex]::IsMatch(
    $newThread,
    'smp_lock::suspend_for_schedule\(\);(?s:.{0,500}?)cpu::wait_for_interrupt\(\);'
)
if ($badSleep) {
    Fail "At least one scheduler sleep still releases the BKL before publishing IDLE."
}

if ($Preview) {
    Write-Host "[PREVIEW] $CpuPath" -ForegroundColor Yellow
    Write-Host "  + prepare_scheduler_idle(): CLI + publish IDLE"
    Write-Host "  + commit_scheduler_idle(): STI;HLT + clear IDLE"
    Write-Host "[PREVIEW] $ThreadPath" -ForegroundColor Yellow
    Write-Host "  + 5 scheduler wait sites converted to race-free handshake"
    Write-Host "  + targeted wake policy unchanged"
    Write-Host "  + P0 v1.3 BKL resume spin unchanged"
    exit 0
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$backupRoot = ".bouchaud-history\backups\.bouchaud-p0-idle-wake-handshake-v14-$stamp"

New-Item -ItemType Directory -Force "$backupRoot\src\arch\x86_64" | Out-Null
New-Item -ItemType Directory -Force "$backupRoot\src\kernel\process" | Out-Null
Copy-Item -LiteralPath $CpuPath -Destination "$backupRoot\$CpuPath" -Force
Copy-Item -LiteralPath $ThreadPath -Destination "$backupRoot\$ThreadPath" -Force

Write-Utf8NoBom $CpuPath $newCpu
Write-Utf8NoBom $ThreadPath $newThread

# Strict UTF-8 decode after write.
$checkCpu = Read-Utf8Strict $CpuPath
$checkThread = Read-Utf8Strict $ThreadPath

if (-not $checkCpu.Contains($V14Marker) -or -not $checkThread.Contains($V14Marker)) {
    Fail "Post-write marker check failed."
}

Write-Host "[PATCH] $CpuPath" -ForegroundColor Green
Write-Host "[PATCH] $ThreadPath" -ForegroundColor Green
Write-Host "[BACKUP] $backupRoot" -ForegroundColor Yellow

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
Write-Host "P0 v1.4 idle wake handshake applied." -ForegroundColor Green
Write-Host "Next:" -ForegroundColor Cyan
Write-Host "  .\VERIFY-P0-IDLE-WAKE-HANDSHAKE-V14.ps1 -Build"
Write-Host "  .\run.ps1"
