param(
    [ValidateRange(512, 4096)]
    [int]$RamMiB = 2048
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot

function Fail([string]$Message) {
    Write-Host ""
    Write-Host "ERREUR: $Message" -ForegroundColor Red
    exit 1
}

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Fail "cargo introuvable"
}

Write-Host "=== Bouchaud Reference Device - foundation bring-up ===" -ForegroundColor Cyan
Write-Host "Ce test est BIOS/legacy. Il ne prouve PAS UEFI/GOP." -ForegroundColor Yellow
Write-Host ""

& cargo bootimage --features reference-bringup
if ($LASTEXITCODE -ne 0) {
    Fail "cargo bootimage --features reference-bringup a echoue"
}

$BootImage = Join-Path `
    $RepoRoot `
    "target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin"

if (-not (Test-Path -LiteralPath $BootImage -PathType Leaf)) {
    Fail "bootimage introuvable: $BootImage"
}

$QemuExe = "C:\Program Files\qemu\qemu-system-x86_64.exe"
if (-not (Test-Path -LiteralPath $QemuExe -PathType Leaf)) {
    Fail "QEMU introuvable: $QemuExe"
}

$qemuArgs = @(
    "-machine", "pc",
    "-drive", "format=raw,file=$BootImage",
    "-m", "$RamMiB",
    "-smp", "1",
    "-serial", "stdio",
    "-net", "none",
    "-accel", "tcg",
    "-cpu", "max",
    "-no-reboot",
    "-no-shutdown"
)

Write-Host "Attendu sur la serie:" -ForegroundColor Cyan
Write-Host "  [BRINGUP] smp=off"
Write-Host "  [BRINGUP] storage-write=off"
Write-Host "  [BRINGUP] network=off"
Write-Host "  BOUCHAUD_REFERENCE_BRINGUP_LEGACY_OK"
Write-Host ""
Write-Host "Fermer QEMU apres observation du marqueur." -ForegroundColor Yellow
Write-Host ""

& $QemuExe @qemuArgs
exit $LASTEXITCODE
