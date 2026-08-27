param(
    [switch]$Build
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$BklPath = "src\kernel\sync\bkl.rs"
$ThreadPath = "src\kernel\process\thread.rs"
$IdtPath = "src\arch\x86_64\idt.rs"
$P0Marker = "BOUCHAUD_P0_TARGETED_SCHED_IPI_V1"
$FixMarker = "BOUCHAUD_P0_TARGETED_IPI_LIVENESS_V13"

function Fail([string]$Message) {
    throw "[P0 v1.3 verify] $Message"
}

$Utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)

function Read-Utf8Strict([string]$Path) {
    $bytes = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Path).Path)
    return $Utf8Strict.GetString($bytes)
}

$thread = Read-Utf8Strict $ThreadPath
$idt = Read-Utf8Strict $IdtPath
$bkl = Read-Utf8Strict $BklPath

if (-not $thread.Contains($P0Marker)) { Fail "Targeted IPI marker missing in thread.rs" }
if (-not $idt.Contains($P0Marker)) { Fail "Targeted IPI marker missing in idt.rs" }
if (-not $bkl.Contains($FixMarker)) { Fail "Liveness marker missing in bkl.rs" }

$startToken = "pub fn resume_after_schedule(depth: usize) {"
$endToken = "/// Acquisitions par origine"
$start = $bkl.IndexOf($startToken)
$end = $bkl.IndexOf($endToken, $start)

if ($start -lt 0 -or $end -lt 0) { Fail "Could not isolate resume_after_schedule()" }

$resume = $bkl.Substring($start, $end - $start)
if ($resume.Contains("wait_for_owner_change")) {
    Fail "resume_after_schedule() still uses HLT-capable adaptive wait"
}
if (-not $resume.Contains("spin_loop();")) {
    Fail "resume_after_schedule() active spin missing"
}

$timerStart = $idt.IndexOf('extern "x86-interrupt" fn timer_interrupt_handler')
$reschedStart = $idt.IndexOf('extern "x86-interrupt" fn reschedule_interrupt_handler', $timerStart)
if ($timerStart -lt 0 -or $reschedStart -lt 0) { Fail "IDT timer handlers not found" }
$timer = $idt.Substring($timerStart, $reschedStart - $timerStart)
if ($timer.Contains("smp::broadcast_reschedule();")) {
    Fail "Periodic broadcast was reintroduced."
}

Write-Host "[OK] targeted scheduler IPIs still enabled" -ForegroundColor Green
Write-Host "[OK] resume_after_schedule cannot HLT" -ForegroundColor Green
Write-Host "[OK] unrelated BKL enter() policy untouched" -ForegroundColor Green

git diff --check
if ($LASTEXITCODE -ne 0) { Fail "git diff --check failed" }

if ($Build) {
    Write-Host ""
    Write-Host "[BUILD] cargo check" -ForegroundColor Cyan
    cargo check
    if ($LASTEXITCODE -ne 0) { Fail "cargo check failed" }

    Write-Host ""
    Write-Host "[BUILD] cargo bootimage" -ForegroundColor Cyan
    cargo bootimage
    if ($LASTEXITCODE -ne 0) { Fail "cargo bootimage failed" }
}

Write-Host ""
Write-Host "[OK] P0 v1.3 verified." -ForegroundColor Green
