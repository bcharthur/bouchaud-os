param(
    [switch]$Build
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ThreadPath = "src\kernel\process\thread.rs"
$IdtPath    = "src\arch\x86_64\idt.rs"
$Marker     = "BOUCHAUD_P0_TARGETED_SCHED_IPI_V1"

function Fail([string]$Message) {
    throw "[P0 v1.2 verify] $Message"
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
        if ($ok) { return $true }
    }
    return $false
}

function Read-Utf8Strict([string]$Path) {
    $bytes = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $Path).Path)
    $utf8 = New-Object System.Text.UTF8Encoding($false, $true)
    return $utf8.GetString($bytes)
}

Write-Host "=== Verify Bouchaud P0 #1 v1.2 ===" -ForegroundColor Cyan

$thread = Read-Utf8Strict $ThreadPath
$idt = Read-Utf8Strict $IdtPath

if (-not $thread.Contains($Marker)) { Fail "Marker missing in thread.rs" }
if (-not $idt.Contains($Marker)) { Fail "Marker missing in idt.rs" }
if (-not $thread.Contains("pub fn running_user_cpu_mask() -> u64")) {
    Fail "running_user_cpu_mask() missing"
}
if (-not $idt.Contains("crate::kernel::task::running_user_cpu_mask()")) {
    Fail "timer does not read targeted CPU mask"
}
if (-not $idt.Contains("smp::reschedule_cpu(cpu);")) {
    Fail "targeted reschedule call missing"
}

# Verify the original non-ASCII identifier still exists as raw UTF-8 bytes.
$ExpectedUtf8Token = [byte[]](0x6E, 0xC5, 0x93, 0x75, 0x64, 0x73)
$threadBytes = [System.IO.File]::ReadAllBytes((Resolve-Path -LiteralPath $ThreadPath).Path)
if (-not (Test-ByteSequence $threadBytes $ExpectedUtf8Token)) {
    Fail "UTF-8 byte-level source integrity check failed."
}

$timerStart = $idt.IndexOf('extern "x86-interrupt" fn timer_interrupt_handler')
$reschedStart = $idt.IndexOf('extern "x86-interrupt" fn reschedule_interrupt_handler', $timerStart)
if ($timerStart -lt 0 -or $reschedStart -lt 0) {
    Fail "Timer handlers not found."
}

$timerBody = $idt.Substring($timerStart, $reschedStart - $timerStart)
if ($timerBody.Contains("smp::broadcast_reschedule();")) {
    Fail "Periodic broadcast still present."
}

Write-Host "[OK] source UTF-8 integrity" -ForegroundColor Green
Write-Host "[OK] targeted scheduler IPI" -ForegroundColor Green
Write-Host "[OK] no periodic broadcast in PIT fallback" -ForegroundColor Green

git diff --check
if ($LASTEXITCODE -ne 0) {
    Fail "git diff --check failed"
}

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
Write-Host "[OK] P0 v1.2 verified." -ForegroundColor Green
