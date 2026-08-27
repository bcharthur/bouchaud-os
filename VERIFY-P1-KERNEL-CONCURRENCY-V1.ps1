param(
    [switch]$Build
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$BklPath      = "src\kernel\sync\bkl.rs"
$WaitQPath    = "src\kernel\sync\wait_queue.rs"
$ThreadPath   = "src\kernel\process\thread.rs"
$LinuxFile    = "src\compat\linux\file.rs"
$LinuxBklPath = "src\compat\linux\bkl.rs"
$VerifierPath = "tools\verifie-verrouillage.py"

function Fail([string]$Message) {
    throw "[P1 verify] $Message"
}

$Utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)
function Read-Utf8Strict([string]$Path) {
    $bytes = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Path).Path)
    return $Utf8Strict.GetString($bytes)
}

$bkl      = Read-Utf8Strict $BklPath
$waitq    = Read-Utf8Strict $WaitQPath
$thread   = Read-Utf8Strict $ThreadPath
$fileRs   = Read-Utf8Strict $LinuxFile
$linuxBklSource = Read-Utf8Strict $LinuxBklPath
$verifier = Read-Utf8Strict $VerifierPath

foreach ($marker in @(
    "BOUCHAUD_P0_TARGETED_SCHED_IPI_V1",
    "BOUCHAUD_P0_IDLE_WAKE_HANDSHAKE_V14",
    "BOUCHAUD_P1_KERNEL_CONCURRENCY_V1"
)) {
    if (-not $thread.Contains($marker)) {
        Fail "thread.rs marker missing: $marker"
    }
}
if (-not $bkl.Contains("BOUCHAUD_P0_TARGETED_IPI_LIVENESS_V13")) {
    Fail "P0 v1.3 marker missing"
}
if (-not $bkl.Contains("BOUCHAUD_P1_KERNEL_CONCURRENCY_V1")) {
    Fail "P1 BKL marker missing"
}

foreach ($needle in @(
    "static BKL_WAITERS: AtomicU64",
    "BKL_WAITERS.fetch_or(bit, Ordering::Release)",
    'core::arch::asm!("sti; hlt"',
    "wake_parked_bkl_waiters(cpu);",
    "pub fn parked_waiter_stats()"
)) {
    if (-not $bkl.Contains($needle)) {
        Fail "BKL handoff element missing: $needle"
    }
}

$wakeReleaseCount = ([regex]::Matches($bkl, 'wake_parked_bkl_waiters\(cpu\);')).Count
if ($wakeReleaseCount -ne 2) {
    Fail "Expected two BKL release wake points, got $wakeReleaseCount"
}

foreach ($needle in @(
    "waiters: AtomicU64",
    "if self.waiters.load(Ordering::Acquire) == 0",
    "self.waiters.fetch_add(1, Ordering::AcqRel)"
)) {
    if (-not $waitq.Contains($needle)) {
        Fail "WaitQueue fast-path element missing: $needle"
    }
}

if (-not $linuxBklSource.Contains('(nr::POLL, "P1 readiness domains')) {
    Fail "POLL missing from SANS_BKL"
}
if (-not $linuxBklSource.Contains('(nr::PPOLL, "P1 readiness domains')) {
    Fail "PPOLL missing from SANS_BKL"
}

$processLocalCount = ([regex]::Matches(
    $fileRs,
    'let process = crate::kernel::abi::processus_courant\(\);'
)).Count
if ($processLocalCount -lt 4) {
    Fail "Expected at least four process-local readiness helpers, got $processLocalCount"
}

if (-not $fileRs.Contains("let _kernel = crate::kernel::smp_lock::enter();")) {
    Fail "Legacy readiness safety boundary missing"
}

if (-not $thread.Contains("MIN_MIGRATION_RESIDENCY_NS: u64 = 250_000_000")) {
    Fail "250ms migration residency missing"
}
if (-not $thread.Contains("now.saturating_add(10_000_000)")) {
    Fail "steal cooldown missing"
}
if (-not $thread.Contains("parked_waiters={:#x} parks={} wake_ipis={}")) {
    Fail "BKL parking telemetry missing"
}

if (-not $verifier.Contains('"POLL": "P1 -- readiness domains')) {
    Fail "named POLL audit missing from verifier"
}
if (-not $verifier.Contains('"compat" / "linux"')) {
    Fail "verifier still points at the pre-restructure ABI path"
}

git diff --check
if ($LASTEXITCODE -ne 0) {
    Fail "git diff --check failed"
}

Write-Host "[OK] P0 targeted wake chain retained" -ForegroundColor Green
Write-Host "[OK] BKL park/release handshake present" -ForegroundColor Green
Write-Host "[OK] WaitQueue zero-waiter fast path present" -ForegroundColor Green
Write-Host "[OK] POLL/PPOLL audited BKL bypass present" -ForegroundColor Green
Write-Host "[OK] migration hysteresis present" -ForegroundColor Green
Write-Host "[OK] BKL parking telemetry present" -ForegroundColor Green

Write-Host ""
Write-Host "[AUDIT] python tools\verifie-verrouillage.py" -ForegroundColor Cyan
python $VerifierPath
if ($LASTEXITCODE -ne 0) {
    Fail "BKL audit verifier failed"
}

if ($Build) {
    Write-Host ""
    Write-Host "[BUILD] cargo check" -ForegroundColor Cyan
    cargo check
    if ($LASTEXITCODE -ne 0) {
        Fail "cargo check failed"
    }

    Write-Host ""
    Write-Host "[BUILD] cargo bootimage" -ForegroundColor Cyan
    cargo bootimage
    if ($LASTEXITCODE -ne 0) {
        Fail "cargo bootimage failed"
    }
}

Write-Host ""
Write-Host "[OK] P1 kernel concurrency verified statically." -ForegroundColor Green
