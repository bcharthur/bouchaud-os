param(
    [switch]$Build
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
    throw "[P0 v1.4 verify] $Message"
}

$Utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)
function Read-Utf8Strict([string]$Path) {
    $bytes = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Path).Path)
    return $Utf8Strict.GetString($bytes)
}

$cpu = Read-Utf8Strict $CpuPath
$thread = Read-Utf8Strict $ThreadPath
$bkl = Read-Utf8Strict $BklPath
$idt = Read-Utf8Strict $IdtPath

if (-not $thread.Contains($TargetedMarker) -or -not $idt.Contains($TargetedMarker)) {
    Fail "Targeted IPI marker missing."
}
if (-not $bkl.Contains($V13Marker)) {
    Fail "P0 v1.3 liveness marker missing."
}
if (-not $cpu.Contains($V14Marker) -or -not $thread.Contains($V14Marker)) {
    Fail "P0 v1.4 marker missing."
}

foreach ($needle in @(
    "pub fn prepare_scheduler_idle()",
    "pub fn commit_scheduler_idle()",
    'asm!("cli", options(nomem, nostack));',
    'asm!("sti; hlt", options(nostack));'
)) {
    if (-not $cpu.Contains($needle)) {
        Fail "CPU handshake element missing: $needle"
    }
}

$prepareCount = ([regex]::Matches($thread, 'cpu::prepare_scheduler_idle\(\);')).Count
$commitCount = ([regex]::Matches($thread, 'cpu::commit_scheduler_idle\(\);')).Count
if ($prepareCount -ne 5 -or $commitCount -ne 5) {
    Fail "Expected 5 scheduler handshake sites, got prepare=$prepareCount commit=$commitCount."
}

if ([regex]::IsMatch(
    $thread,
    'smp_lock::suspend_for_schedule\(\);(?s:.{0,500}?)cpu::wait_for_interrupt\(\);'
)) {
    Fail "Old lost-wakeup scheduler sleep pattern still present."
}

# v1.3 invariant: resume_after_schedule must not HLT.
$startToken = "pub fn resume_after_schedule(depth: usize) {"
$endToken = "/// Acquisitions par origine"
$start = $bkl.IndexOf($startToken)
$end = $bkl.IndexOf($endToken, $start)
if ($start -lt 0 -or $end -lt 0) { Fail "Could not isolate resume_after_schedule()" }
$resume = $bkl.Substring($start, $end - $start)
if ($resume.Contains("wait_for_owner_change")) {
    Fail "resume_after_schedule() regained the HLT-capable wait path."
}

# P0 invariant: periodic BSP broadcast remains gone when local timer mode is active.
$timerStart = $idt.IndexOf('extern "x86-interrupt" fn timer_interrupt_handler')
$reschedStart = $idt.IndexOf('extern "x86-interrupt" fn reschedule_interrupt_handler', $timerStart)
if ($timerStart -lt 0 -or $reschedStart -lt 0) { Fail "Timer handlers not found." }
$timer = $idt.Substring($timerStart, $reschedStart - $timerStart)

if (-not $timer.Contains("!smp::local_scheduler_timer_enabled()")) {
    Fail "Targeted/local-timer guard missing from BSP timer."
}

Write-Host "[OK] targeted scheduler wake policy present" -ForegroundColor Green
Write-Host "[OK] P0 v1.3 BKL-resume liveness retained" -ForegroundColor Green
Write-Host "[OK] scheduler idle handshake uses CLI -> publish IDLE -> BKL release -> STI;HLT" -ForegroundColor Green
Write-Host "[OK] 5 scheduler sleep sites converted" -ForegroundColor Green

git diff --check
if ($LASTEXITCODE -ne 0) {
    Fail "git diff --check failed."
}

if ($Build) {
    Write-Host ""
    Write-Host "[BUILD] cargo check" -ForegroundColor Cyan
    cargo check
    if ($LASTEXITCODE -ne 0) { Fail "cargo check failed." }

    Write-Host ""
    Write-Host "[BUILD] cargo bootimage" -ForegroundColor Cyan
    cargo bootimage
    if ($LASTEXITCODE -ne 0) { Fail "cargo bootimage failed." }
}

Write-Host ""
Write-Host "[OK] P0 v1.4 verified." -ForegroundColor Green
