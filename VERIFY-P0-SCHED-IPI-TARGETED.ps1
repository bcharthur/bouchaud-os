param(
    [switch]$Build
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$ThreadPath = "src\kernel\process\thread.rs"
$IdtPath    = "src\arch\x86_64\idt.rs"
$Marker     = "BOUCHAUD_P0_TARGETED_SCHED_IPI_V1"

$Utf8Strict = New-Object System.Text.UTF8Encoding($false, $true)

function Read-Utf8Strict([string]$Path) {
    return [System.IO.File]::ReadAllText((Resolve-Path -LiteralPath $Path).Path, $Utf8Strict)
}

Write-Host "=== Verification P0 targeted scheduler IPI v1.1 ===" -ForegroundColor Cyan

$thread = Read-Utf8Strict $ThreadPath
$idt = Read-Utf8Strict $IdtPath

if (-not $thread.Contains($Marker)) { throw "Marqueur P0 absent de thread.rs" }
if (-not $idt.Contains($Marker)) { throw "Marqueur P0 absent de idt.rs" }
if (-not $thread.Contains("pub fn running_user_cpu_mask() -> u64")) { throw "running_user_cpu_mask absent" }
if (-not $idt.Contains("smp::reschedule_cpu(cpu);")) { throw "IPI cible absent" }

if ($thread.Contains("nÅ") -or $thread.Contains("â€")) {
    throw "Mojibake detecte dans thread.rs"
}
if (-not $thread.Contains("let nœuds:")) {
    throw "Token UTF-8 de controle 'nœuds' absent"
}
Write-Host "[OK] UTF-8 strict / aucun mojibake connu" -ForegroundColor Green
Write-Host "[OK] P0 targeted IPI present" -ForegroundColor Green

$timerStart = $idt.IndexOf('extern "x86-interrupt" fn timer_interrupt_handler')
$reschedStart = $idt.IndexOf('extern "x86-interrupt" fn reschedule_interrupt_handler', $timerStart)
if ($timerStart -lt 0 -or $reschedStart -lt 0) { throw "Handlers IDT introuvables" }
$timerBody = $idt.Substring($timerStart, $reschedStart - $timerStart)
if ($timerBody.Contains("smp::broadcast_reschedule();")) {
    throw "Broadcast periodique encore present dans le handler timer"
}
Write-Host "[OK] aucun broadcast periodique du PIT" -ForegroundColor Green

git diff --check
if ($LASTEXITCODE -ne 0) { throw "git diff --check a echoue" }

if ($Build) {
    Write-Host ""
    Write-Host "[BUILD] cargo check" -ForegroundColor Cyan
    cargo check
    if ($LASTEXITCODE -ne 0) { throw "cargo check a echoue" }

    Write-Host ""
    Write-Host "[BUILD] cargo bootimage" -ForegroundColor Cyan
    cargo bootimage
    if ($LASTEXITCODE -ne 0) { throw "cargo bootimage a echoue" }
}

Write-Host ""
Write-Host "[OK] validation terminee." -ForegroundColor Green
