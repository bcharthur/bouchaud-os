param(
    [switch]$Preview,
    [switch]$SkipCargoCheck
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$BklPath      = "src\kernel\sync\bkl.rs"
$WaitQPath    = "src\kernel\sync\wait_queue.rs"
$ThreadPath   = "src\kernel\process\thread.rs"
$LinuxFile    = "src\compat\linux\file.rs"
$LinuxBklPath = "src\compat\linux\bkl.rs"
$VerifierPath = "tools\verifie-verrouillage.py"

$P0Targeted = "BOUCHAUD_P0_TARGETED_SCHED_IPI_V1"
$P0V13      = "BOUCHAUD_P0_TARGETED_IPI_LIVENESS_V13"
$P0V14      = "BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14"
$P1Marker   = "BOUCHAUD_P1_KERNEL_CONCURRENCY_V1"
$PatchScriptVersion = "1.1-night-checkpoint"

function Fail([string]$Message) {
    throw "[Bouchaud P1 concurrency] $Message"
}

if (-not (Test-Path ".git")) {
    Fail "Run this script from the bouchaud-os repository root."
}

foreach ($path in @($BklPath, $WaitQPath, $ThreadPath, $LinuxFile, $LinuxBklPath, $VerifierPath)) {
    if (-not (Test-Path -LiteralPath $path)) {
        Fail "Missing file: $path"
    }
}

$Utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)
$Utf8NoBom  = New-Object System.Text.UTF8Encoding($false)

function Read-Utf8Strict([string]$Path) {
    $full = (Resolve-Path -LiteralPath $Path).Path
    $bytes = [System.IO.File]::ReadAllBytes($full)
    return $Utf8Strict.GetString($bytes)
}

function Write-Utf8NoBom([string]$Path, [string]$Content) {
    $full = (Resolve-Path -LiteralPath $Path).Path
    [System.IO.File]::WriteAllText($full, $Content, $Utf8NoBom)
}

function Replace-ExactOnce(
    [string]$Text,
    [string]$Old,
    [string]$New,
    [string]$Label
) {
    $first = $Text.IndexOf($Old, [System.StringComparison]::Ordinal)
    if ($first -lt 0) {
        Fail "Anchor not found: $Label"
    }
    $second = $Text.IndexOf($Old, $first + $Old.Length, [System.StringComparison]::Ordinal)
    if ($second -ge 0) {
        Fail "Anchor is not unique: $Label"
    }
    return $Text.Substring(0, $first) + $New + $Text.Substring($first + $Old.Length)
}

function Replace-RegexOnce(
    [string]$Text,
    [string]$Pattern,
    [string]$Replacement,
    [string]$Label
) {
    $regex = [regex]::new($Pattern)
    $matches = $regex.Matches($Text)
    if ($matches.Count -ne 1) {
        Fail "Regex anchor '$Label' matched $($matches.Count) time(s)"
    }
    return $regex.Replace(
        $Text,
        [System.Text.RegularExpressions.MatchEvaluator]{ param($m) $Replacement },
        1
    )
}

function Replace-FirstAfter(
    [string]$Text,
    [string]$Anchor,
    [string]$Old,
    [string]$New,
    [string]$Label
) {
    $start = $Text.IndexOf($Anchor, [System.StringComparison]::Ordinal)
    if ($start -lt 0) {
        Fail "Function anchor not found: $Label"
    }
    $pos = $Text.IndexOf($Old, $start, [System.StringComparison]::Ordinal)
    if ($pos -lt 0) {
        Fail "Replacement anchor not found after function: $Label"
    }
    return $Text.Substring(0, $pos) + $New + $Text.Substring($pos + $Old.Length)
}

$bkl      = Read-Utf8Strict $BklPath
$waitq    = Read-Utf8Strict $WaitQPath
$thread   = Read-Utf8Strict $ThreadPath
$fileRs   = Read-Utf8Strict $LinuxFile
$linuxBklSource = Read-Utf8Strict $LinuxBklPath
$verifier = Read-Utf8Strict $VerifierPath

# ---------------------------------------------------------------------------
# Preconditions: P1 is deliberately built on top of the validated P0 chain.
# ---------------------------------------------------------------------------

if (-not $thread.Contains($P0Targeted)) {
    Fail "P0 targeted scheduler IPI marker missing from thread.rs"
}
if (-not $thread.Contains($P0V14)) {
    Fail "P0 v1.4 idle/wake handshake marker missing from thread.rs"
}
if (-not $bkl.Contains($P0V13)) {
    Fail "P0 v1.3 BKL resume liveness marker missing from bkl.rs"
}

if ($bkl.Contains($P1Marker)) {
    Write-Host "[OK] P1 concurrency patch already applied." -ForegroundColor Green
    exit 0
}

# ===========================================================================
# A. BKL: race-free parking + explicit wakeup on release
# ===========================================================================
#
# The old adaptive BKL path could HLT after a short spin and relied on the old
# periodic scheduler IPI to wake it. P0 intentionally removed that heartbeat.
#
# P1 publishes a waiter bit with IF=0, rechecks OWNER, then STI;HLT. A release
# stores FREE first and wakes every parked contender. This is the same lost-
# wake avoidance principle as the scheduler idle handshake validated in P0 v1.4.
# ===========================================================================

$depthAnchor = 'static DEPTH: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(0) }; MAX_CPUS];'
$depthReplacement = @'
static DEPTH: [AtomicUsize; MAX_CPUS] = [const { AtomicUsize::new(0) }; MAX_CPUS];

// BOUCHAUD_P1_KERNEL_CONCURRENCY_V1
//
// CPUs that have deliberately parked while waiting for the BKL. Publishing the
// bit while IF=0, then rechecking OWNER, closes the release-before-HLT race.
static BKL_WAITERS: AtomicU64 = AtomicU64::new(0);
static BKL_PARKS: AtomicU64 = AtomicU64::new(0);
static BKL_WAKE_IPIS: AtomicU64 = AtomicU64::new(0);
'@
$bkl = Replace-ExactOnce $bkl $depthAnchor $depthReplacement "BKL waiter state"

$adaptivePattern = '(?ms)// BOUCHAUD_BKL_ADAPTIVE_IDLE.*?^fn wait_for_owner_change\(active_spins: &mut usize\) \{.*?^\}\r?\n'
$adaptiveReplacement = @'
// BOUCHAUD_BKL_ADAPTIVE_IDLE
// BOUCHAUD_P1_KERNEL_CONCURRENCY_V1
//
// Keep short critical sections cheap, but never rely on a scheduler heartbeat
// to wake a parked BKL contender. P0 made scheduler IPIs event-driven, so BKL
// release itself now owns the wakeup contract.
const BKL_ACTIVE_SPINS: usize = 512;

#[inline]
fn wake_parked_bkl_waiters(releasing_cpu: usize) {
    let waiters = BKL_WAITERS.load(Ordering::Acquire);
    if waiters == 0 {
        return;
    }

    let online = crate::arch::x86_64::smp::schedulable_cpus()
        .min(MAX_CPUS)
        .min(64);
    let mut target = 0usize;
    while target < online {
        let bit = 1u64 << target;
        if target != releasing_cpu && waiters & bit != 0 {
            crate::arch::x86_64::smp::reschedule_cpu(target);
            BKL_WAKE_IPIS.fetch_add(1, Ordering::Relaxed);
        }
        target += 1;
    }
}

#[inline]
fn wait_for_owner_change(active_spins: &mut usize) {
    if *active_spins < BKL_ACTIVE_SPINS {
        *active_spins += 1;
        spin_loop();
        return;
    }
    *active_spins = 0;

    // Never enable interrupts from a context that arrived with IF=0.
    if !interrupts::are_enabled() {
        spin_loop();
        return;
    }

    let cpu_id = cpu();
    let bit = 1u64 << cpu_id;

    // Publish the sleep intent with interrupts disabled. If the owner released
    // before our publication, the recheck sees FREE and we do not sleep. If it
    // releases after the recheck, it sees our bit and sends an IPI. Because IF
    // remains zero until STI;HLT, that IPI cannot be consumed before HLT.
    interrupts::disable();
    BKL_WAITERS.fetch_or(bit, Ordering::Release);

    if OWNER.load(Ordering::Acquire) == FREE {
        BKL_WAITERS.fetch_and(!bit, Ordering::AcqRel);
        interrupts::enable();
        return;
    }

    BKL_PARKS.fetch_add(1, Ordering::Relaxed);
    unsafe {
        core::arch::asm!("sti; hlt", options(nomem, nostack));
    }
    BKL_WAITERS.fetch_and(!bit, Ordering::AcqRel);
}
'@
$bkl = Replace-RegexOnce $bkl $adaptivePattern $adaptiveReplacement "adaptive BKL wait"

# Wake parked waiters from both real release paths: normal guard drop and
# scheduler suspension.
$releaseNormalOld = @'
    probe_note_release(cpu, 1);
    OWNER.store(FREE, Ordering::Release);
'@
$releaseNormalNew = @'
    probe_note_release(cpu, 1);
    OWNER.store(FREE, Ordering::Release);
    wake_parked_bkl_waiters(cpu);
'@
$bkl = Replace-ExactOnce $bkl $releaseNormalOld $releaseNormalNew "normal BKL release wake"

$releaseSuspendOld = @'
    probe_note_release(cpu, 2);
    OWNER.store(FREE, Ordering::Release);
    depth
'@
$releaseSuspendNew = @'
    probe_note_release(cpu, 2);
    OWNER.store(FREE, Ordering::Release);
    wake_parked_bkl_waiters(cpu);
    depth
'@
$bkl = Replace-ExactOnce $bkl $releaseSuspendOld $releaseSuspendNew "scheduler BKL release wake"

# Better max-hold attribution. The live site is often cleared before Drop;
# fall back to the site captured when the ownership provenance was published.
$siteOld = @'
        PLUS_LONGUE_TENUE_SITE.store(
            crate::kernel::task::stall_site_de_la_tenue(),
            Ordering::Relaxed,
        );
'@
$siteNew = @'
        let measured_site = crate::kernel::task::stall_site_de_la_tenue();
        let attributed_site = if measured_site != 0 {
            measured_site
        } else {
            PROBE_OWNER_SITE.load(Ordering::Acquire)
        };
        PLUS_LONGUE_TENUE_SITE.store(attributed_site, Ordering::Relaxed);
'@
$bkl = Replace-ExactOnce $bkl $siteOld $siteNew "max BKL hold attribution"

$statsAnchor = 'pub fn contention_stats() -> (u64, u64, u64) {'
$statsInsert = @'
// BOUCHAUD_P1_KERNEL_CONCURRENCY_V1
pub fn parked_waiter_stats() -> (u64, u64, u64) {
    (
        BKL_WAITERS.load(Ordering::Acquire),
        BKL_PARKS.load(Ordering::Relaxed),
        BKL_WAKE_IPIS.load(Ordering::Relaxed),
    )
}

pub fn contention_stats() -> (u64, u64, u64) {
'@
$bkl = Replace-ExactOnce $bkl $statsAnchor $statsInsert "BKL parked waiter stats"

# ===========================================================================
# B. WaitQueue: do not enter/scan the BKL when nobody is actually waiting.
# ===========================================================================

$waitStructOld = @'
pub struct WaitQueue {
    generation: AtomicU64,
}
'@
$waitStructNew = @'
pub struct WaitQueue {
    generation: AtomicU64,
    // BOUCHAUD_P1_KERNEL_CONCURRENCY_V1
    // Fast-path hint. Generation remains the correctness mechanism; this
    // counter only avoids a pointless BKL acquisition when there is no waiter.
    waiters: AtomicU64,
}
'@
$waitq = Replace-ExactOnce $waitq $waitStructOld $waitStructNew "WaitQueue waiter counter"

$waitCtorOld = 'Self { generation: AtomicU64::new(1) }'
$waitCtorNew = 'Self { generation: AtomicU64::new(1), waiters: AtomicU64::new(0) }'
$waitq = Replace-ExactOnce $waitq $waitCtorOld $waitCtorNew "WaitQueue constructor"

$waitPattern = '(?ms)^    pub fn wait\(&self, ticket: WaitTicket\) \{.*?^    \}'
$waitReplacement = @'
    pub fn wait(&self, ticket: WaitTicket) {
        if self.generation.load(Ordering::Acquire) != ticket.0 {
            return;
        }

        self.waiters.fetch_add(1, Ordering::AcqRel);
        if self.generation.load(Ordering::Acquire) != ticket.0 {
            self.waiters.fetch_sub(1, Ordering::AcqRel);
            return;
        }

        let _kernel = enter_bkl();
        if self.generation.load(Ordering::Acquire) != ticket.0 {
            self.waiters.fetch_sub(1, Ordering::AcqRel);
            return;
        }

        crate::kernel::task::park_current_on(self.key());
        self.waiters.fetch_sub(1, Ordering::AcqRel);
    }
'@
$waitq = Replace-RegexOnce $waitq $waitPattern $waitReplacement "WaitQueue::wait"

$waitUntilPattern = '(?ms)^    pub fn wait_until\(&self, ticket: WaitTicket, deadline_ns: u64\) -> bool \{.*?^    \}'
$waitUntilReplacement = @'
    pub fn wait_until(&self, ticket: WaitTicket, deadline_ns: u64) -> bool {
        if self.generation.load(Ordering::Acquire) != ticket.0 {
            return true;
        }

        self.waiters.fetch_add(1, Ordering::AcqRel);
        if self.generation.load(Ordering::Acquire) != ticket.0 {
            self.waiters.fetch_sub(1, Ordering::AcqRel);
            return true;
        }

        let _kernel = enter_bkl();
        if self.generation.load(Ordering::Acquire) != ticket.0 {
            self.waiters.fetch_sub(1, Ordering::AcqRel);
            return true;
        }

        let notified = crate::kernel::task::park_current_on_until(self.key(), deadline_ns);
        self.waiters.fetch_sub(1, Ordering::AcqRel);
        notified
    }
'@
$waitq = Replace-RegexOnce $waitq $waitUntilPattern $waitUntilReplacement "WaitQueue::wait_until"

$wakeOnePattern = '(?ms)^    pub fn wake_one\(&self\) -> bool \{.*?^    \}'
$wakeOneReplacement = @'
    pub fn wake_one(&self) -> bool {
        self.generation.fetch_add(1, Ordering::Release);
        if self.waiters.load(Ordering::Acquire) == 0 {
            return false;
        }
        let _kernel = enter_bkl();
        crate::kernel::task::wake_wait_queue(self.key(), 1) != 0
    }
'@
$waitq = Replace-RegexOnce $waitq $wakeOnePattern $wakeOneReplacement "WaitQueue::wake_one"

$wakeAllPattern = '(?ms)^    pub fn wake_all\(&self\) -> usize \{.*?^    \}'
$wakeAllReplacement = @'
    pub fn wake_all(&self) -> usize {
        self.generation.fetch_add(1, Ordering::Release);
        if self.waiters.load(Ordering::Acquire) == 0 {
            return 0;
        }
        let _kernel = enter_bkl();
        crate::kernel::task::wake_wait_queue(self.key(), usize::MAX)
    }
'@
$waitq = Replace-RegexOnce $waitq $wakeAllPattern $wakeAllReplacement "WaitQueue::wake_all"

# ===========================================================================
# C. poll/ppoll: scan readiness without holding the global kernel lock.
#
# User memory is already protected by the process Mm domain. The descriptor
# table has its own SpinLock. Pipes/socketpairs/eventfd/timerfd have object
# locks. Legacy keyboard/input and the TCP pump are still wrapped in a short
# explicit BKL section so this patch does not invent unsynchronized driver use.
# The BKL is reacquired only when the task actually parks in WaitQueue.
# ===========================================================================

foreach ($fn in @(
    "fn writable(fd: i32) -> bool {",
    "fn etat_pair(fd: i32) -> u32 {",
    "fn readable(fd: i32) -> bool {",
    "fn readiness_deadline_ns(fd: i32) -> Option<u64> {"
)) {
    $fileRs = Replace-FirstAfter `
        $fileRs `
        $fn `
        "let process = task::current_process();" `
        "let process = crate::kernel::abi::processus_courant();" `
        "process-local readiness in $fn"
}

$consoleOld = '        FdKind::Console => keyboard::has_pending(),'
$consoleNew = @'
        FdKind::Console => {
            let _kernel = crate::kernel::smp_lock::enter();
            keyboard::has_pending()
        }
'@
$fileRs = Replace-ExactOnce $fileRs $consoleOld $consoleNew "console readiness guard"

$kbdOld = '        FdKind::InputKeyboard => input::keyboard_pending(),'
$kbdNew = @'
        FdKind::InputKeyboard => {
            let _kernel = crate::kernel::smp_lock::enter();
            input::keyboard_pending()
        }
'@
$fileRs = Replace-ExactOnce $fileRs $kbdOld $kbdNew "keyboard readiness guard"

$mouseOld = '        FdKind::InputMouse => input::mouse_pending(),'
$mouseNew = @'
        FdKind::InputMouse => {
            let _kernel = crate::kernel::smp_lock::enter();
            input::mouse_pending()
        }
'@
$fileRs = Replace-ExactOnce $fileRs $mouseOld $mouseNew "mouse readiness guard"

$socketOld = '        FdKind::Socket(state) => crate::kernel::abi::net::socket_readable(&state),'
$socketNew = @'
        FdKind::Socket(state) => {
            // TcpConn::pump still reaches the legacy global NIC/network path.
            // Keep that tiny part serialized until the NIC/network driver gets
            // its own synchronization domain.
            let _kernel = crate::kernel::smp_lock::enter();
            crate::kernel::abi::net::socket_readable(&state)
        }
'@
$fileRs = Replace-ExactOnce $fileRs $socketOld $socketNew "socket readiness guard"

# Put poll and ppoll in the audited BKL-bypass table.
$timeAnchor = '    (nr::TIME, "ancre d''epoque atomique + Mm"),'
$pollAudit = @'
    (nr::TIME, "ancre d'epoque atomique + Mm"),
    // --- Readiness: descriptor/object domains + BKL only at park/legacy I/O -
    //
    // poll/ppoll no longer keep the BKL across the entire readiness scan and
    // sleep. FileTable, Mm and waitable objects own their synchronization.
    // The few legacy readiness probes that still touch PS/2 or TcpConn::pump
    // take a short explicit BKL in file.rs. WaitQueue reacquires the BKL only
    // around the TASKS state transition needed to park/wake the current task.
    (nr::POLL, "P1 readiness domains; BKL only for legacy probe and park boundary"),
    (nr::PPOLL, "P1 readiness domains; BKL only for legacy probe and park boundary"),
'@
$linuxBklSource = Replace-ExactOnce $linuxBklSource $timeAnchor $pollAudit "POLL/PPOLL BKL audit"

# ===========================================================================
# D. Scheduler locality: stop cache/TLB ping-pong.
# ===========================================================================
#
# 20 ms was far below the useful cache residency seen in the Ladybird run.
# Keep a migrated thread on its CPU for 250 ms, and add a small cooldown after
# a successful/failed steal. Local ready work is always consumed first.
# ===========================================================================

if (([regex]::Matches($thread, 'const MIN_MIGRATION_RESIDENCY_NS: u64 = 20_000_000;')).Count -ne 1) {
    Fail "Expected exactly one 20ms migration residency constant"
}
$thread = $thread.Replace(
    'const MIN_MIGRATION_RESIDENCY_NS: u64 = 20_000_000;',
    '// BOUCHAUD_P1_KERNEL_CONCURRENCY_V1`r`n    const MIN_MIGRATION_RESIDENCY_NS: u64 = 250_000_000;'
)
# The literal above intentionally needs a real newline, not PowerShell escape
# text embedded inside Rust.
$thread = $thread.Replace(
    '// BOUCHAUD_P1_KERNEL_CONCURRENCY_V1`r`n    const MIN_MIGRATION_RESIDENCY_NS',
    "// BOUCHAUD_P1_KERNEL_CONCURRENCY_V1`r`n    const MIN_MIGRATION_RESIDENCY_NS"
)

$retryOld = 'STEAL_RETRY_AFTER_NS[cpu].store(now.saturating_add(2_000_000), Ordering::Relaxed);'
$retryMatches = ([regex]::Matches($thread, [regex]::Escape($retryOld))).Count
if ($retryMatches -lt 1) {
    Fail "2ms steal retry anchor missing"
}
# Only the first occurrence belongs to the invalid-stolen-candidate path in
# pick_next. Replacing it by index avoids touching unrelated timer constants.
$retryPos = $thread.IndexOf($retryOld, [System.StringComparison]::Ordinal)
$retryNew = 'STEAL_RETRY_AFTER_NS[cpu].store(now.saturating_add(10_000_000), Ordering::Relaxed);'
$thread = $thread.Substring(0, $retryPos) + $retryNew + $thread.Substring($retryPos + $retryOld.Length)

$successOld = 'STEAL_RETRY_AFTER_NS[cpu].store(0, Ordering::Relaxed);'
$successNew = @'
STEAL_RETRY_AFTER_NS[cpu].store(
        now.saturating_add(10_000_000),
        Ordering::Relaxed,
    );
'@
$thread = Replace-ExactOnce $thread $successOld $successNew "successful steal cooldown"

# Add BKL parking counters to the existing compact BKL-STATS line.
$bklStatsOld = @'
    let (max_tenue, max_site) = smp_lock::plus_longue_tenue();
    crate::kernel::dmesg::log_fmt(format_args!(
        "[BKL-STATS] wait_ns={} hold_ns={} acquisitions={} enter={} try_enter={} resume={} max_hold_ns={} max_hold_site={} preempt_irq_bkl_tenu={} identite_repli={}",
        bkl_wait, bkl_hold, bkl_acq, acq_enter, acq_try, acq_resume,
        max_tenue, max_site, preempt_irq_bkl_tenu(), identite_repli(),
    ));
'@
$bklStatsNew = @'
    let (max_tenue, max_site) = smp_lock::plus_longue_tenue();
    let (parked_waiters, bkl_parks, bkl_wake_ipis) = smp_lock::parked_waiter_stats();
    crate::kernel::dmesg::log_fmt(format_args!(
        "[BKL-STATS] wait_ns={} hold_ns={} acquisitions={} enter={} try_enter={} resume={} max_hold_ns={} max_hold_site={} preempt_irq_bkl_tenu={} identite_repli={} parked_waiters={:#x} parks={} wake_ipis={}",
        bkl_wait, bkl_hold, bkl_acq, acq_enter, acq_try, acq_resume,
        max_tenue, max_site, preempt_irq_bkl_tenu(), identite_repli(),
        parked_waiters, bkl_parks, bkl_wake_ipis,
    ));
'@
$thread = Replace-ExactOnce $thread $bklStatsOld $bklStatsNew "BKL stats extension"

# ===========================================================================
# E. Keep the external BKL audit valid after the multiplatform restructure.
# ===========================================================================

$verifier = $verifier.Replace('src/kernel/abi', 'src/compat/linux')
$verifier = $verifier.Replace(
    'RACINE / "src" / "kernel" / "abi" / "nr.rs"',
    'RACINE / "src" / "compat" / "linux" / "nr.rs"'
)
$verifier = $verifier.Replace(
    'RACINE / "src" / "kernel" / "abi" / "bkl.rs"',
    'RACINE / "src" / "compat" / "linux" / "bkl.rs"'
)
$verifier = $verifier.Replace(
    'RACINE / "src" / "kernel" / "abi" / "mod.rs"',
    'RACINE / "src" / "compat" / "linux" / "mod.rs"'
)

$auditAnchor = '    "TIME": "A1 lot 2 -- ancre d''epoque atomique + Mm",'
$auditReplacement = @'
    "TIME": "A1 lot 2 -- ancre d'epoque atomique + Mm",
    # P1 -- poll/ppoll scan the process-local FileTable and object locks.
    # Legacy PS/2/TCP readiness probes explicitly reacquire the BKL, and the
    # WaitQueue park transition reacquires it at the scheduler boundary.
    "POLL": "P1 -- readiness domains + explicit legacy/park BKL boundaries",
    "PPOLL": "P1 -- readiness domains + explicit legacy/park BKL boundaries",
'@
$verifier = Replace-ExactOnce $verifier $auditAnchor $auditReplacement "POLL/PPOLL named verifier audits"

# ---------------------------------------------------------------------------
# Preview / backup / write / static validation
# ---------------------------------------------------------------------------

if ($Preview) {
    Write-Host "=== P1 preview ===" -ForegroundColor Cyan
    Write-Host "BKL       : race-free parked waiter handoff; 512-spin threshold; max-site fallback"
    Write-Host "WaitQueue : zero-waiter notification fast path"
    Write-Host "poll      : POLL/PPOLL outer BKL removed; legacy probes remain explicitly serialized"
    Write-Host "scheduler : migration residency 20ms -> 250ms; 10ms steal cooldown"
    Write-Host "audit     : verifier paths fixed for src/compat/linux + named POLL/PPOLL audits"
    Write-Host "metrics   : BKL-STATS gains parked_waiters/parks/wake_ipis"
    Write-Host ""
    Write-Host "No file modified." -ForegroundColor Yellow
    exit 0
}

$stamp = Get-Date -Format "yyyyMMdd-HHmmss"
$backupRoot = ".bouchaud-history\backups\.bouchaud-p1-kernel-concurrency-v1-$stamp"

foreach ($file in @($BklPath, $WaitQPath, $ThreadPath, $LinuxFile, $LinuxBklPath, $VerifierPath)) {
    $dest = Join-Path $backupRoot $file
    $parent = Split-Path -Parent $dest
    New-Item -ItemType Directory -Force $parent | Out-Null
    Copy-Item -LiteralPath $file -Destination $dest -Force
}

Write-Utf8NoBom $BklPath $bkl
Write-Utf8NoBom $WaitQPath $waitq
Write-Utf8NoBom $ThreadPath $thread
Write-Utf8NoBom $LinuxFile $fileRs
Write-Utf8NoBom $LinuxBklPath $linuxBklSource
Write-Utf8NoBom $VerifierPath $verifier

Write-Host "[PATCH] $BklPath" -ForegroundColor Green
Write-Host "[PATCH] $WaitQPath" -ForegroundColor Green
Write-Host "[PATCH] $ThreadPath" -ForegroundColor Green
Write-Host "[PATCH] $LinuxFile" -ForegroundColor Green
Write-Host "[PATCH] $LinuxBklPath" -ForegroundColor Green
Write-Host "[PATCH] $VerifierPath" -ForegroundColor Green
Write-Host "[BACKUP] $backupRoot" -ForegroundColor Yellow

# Strict decoding catches accidental Windows encoding damage before Rust sees it.
foreach ($file in @($BklPath, $WaitQPath, $ThreadPath, $LinuxFile, $LinuxBklPath, $VerifierPath)) {
    [void](Read-Utf8Strict $file)
}

git diff --check
if ($LASTEXITCODE -ne 0) {
    Fail "git diff --check failed. Restore from $backupRoot"
}

Write-Host ""
Write-Host "[AUDIT] python tools\verifie-verrouillage.py" -ForegroundColor Cyan
python $VerifierPath
if ($LASTEXITCODE -ne 0) {
    Fail "BKL audit verifier failed. Restore from $backupRoot"
}

if (-not $SkipCargoCheck) {
    Write-Host ""
    Write-Host "[BUILD] cargo check" -ForegroundColor Cyan
    cargo check
    if ($LASTEXITCODE -ne 0) {
        Fail "cargo check failed. Restore from $backupRoot"
    }
}

Write-Host ""
Write-Host "P1 kernel concurrency patch applied." -ForegroundColor Green
Write-Host "Next:" -ForegroundColor Cyan
Write-Host "  .\VERIFY-P1-KERNEL-CONCURRENCY-V1.ps1 -Build"
Write-Host "  .\run.ps1"
