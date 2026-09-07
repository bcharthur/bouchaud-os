param(
    [ValidateRange(4096, 16384)]
    [int]$RamMiB = 12288,

    [ValidateSet("tcg", "whpx")]
    [string]$Accel = "tcg",

    [switch]$Fullscreen,
    [switch]$SkipPrepare
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot

function Fail([string]$Message) {
    Write-Host ""
    Write-Host "ERREUR: $Message" -ForegroundColor Red
    exit 1
}

if (-not $SkipPrepare) {
    & ".\tools\reference\prepare-reference-ladybird.ps1"
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

& ".\tools\reference\build-reference-stage2.ps1"
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$Image = Join-Path $RepoRoot "target\reference\bouchaud-reference-stage2.img"
$LadybirdImage = Join-Path $RepoRoot "ladybird-browser.img"

if (-not (Test-Path -LiteralPath $Image -PathType Leaf)) {
    Fail "image Stage 2 absente: $Image"
}
if (-not (Test-Path -LiteralPath $LadybirdImage -PathType Leaf)) {
    Fail "ladybird-browser.img absent"
}

& python ".\tools\reference\verify-reference-ladybird-image.py" $LadybirdImage
if ($LASTEXITCODE -ne 0) {
    Fail "ladybird-browser.img invalide"
}

$QemuCandidates = @(
    "C:\Program Files\qemu\qemu-system-x86_64.exe",
    "C:\Program Files (x86)\qemu\qemu-system-x86_64.exe"
)
$QemuExe = $QemuCandidates | Where-Object {
    Test-Path -LiteralPath $_ -PathType Leaf
} | Select-Object -First 1

if (-not $QemuExe) {
    $QemuCmd = Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue
    if ($QemuCmd) { $QemuExe = $QemuCmd.Source }
}
if (-not $QemuExe) { Fail "qemu-system-x86_64 introuvable" }

$Share = Join-Path (Split-Path -Parent $QemuExe) "share"
$OvmfCode = @(
    (Join-Path $Share "edk2-x86_64-code.fd"),
    (Join-Path $Share "edk2-x86_64-code-secure.fd")
) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1

$OvmfVars = @(
    (Join-Path $Share "edk2-i386-vars.fd"),
    (Join-Path $Share "edk2-x86_64-vars.fd")
) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1

if (-not $OvmfCode) { Fail "OVMF CODE introuvable" }
if (-not $OvmfVars) { Fail "OVMF VARS introuvable" }

Write-Host ""
Write-Host "=== Bouchaud Stage 2 FINAL V2 / Ladybird ===" -ForegroundColor Cyan
Write-Host "Machine     : i440fx/PIIX (2 disques IDE visibles par le pilote ATA PIO)"
Write-Host "RAM         : $RamMiB MiB"
Write-Host "vCPU        : 1 (BSP only)"
Write-Host "Reseau      : QEMU e1000 + SLIRP"
Write-Host "Boot        : $Image"
Write-Host "Ladybird    : $LadybirdImage"
Write-Host "Accel       : $Accel"
Write-Host ""
Write-Host "Attendu en serie:" -ForegroundColor Yellow
Write-Host "  BOUCHAUD_STAGE2_NATIVE_VIEWPORT_OK"
Write-Host "  BOUCHAUD_STAGE2_LADYBIRD_RUNTIME_OK"
Write-Host "  BOUCHAUD_STAGE2_DESKTOP_TASK_ENTER"
Write-Host "  BOUCHAUD_STAGE2_INPUT_READY"
Write-Host "  BOUCHAUD_STAGE2_WINDOW_MANAGER_READY"
Write-Host "Puis double-clic Ladybird:"
Write-Host "  BOUCHAUD_STAGE2_LADYBIRD_LAUNCHED pid=..."
Write-Host ""

$Args = @(
    "-machine", "pc",
    "-m", "$RamMiB",
    "-smp", "1",
    "-drive", "if=pflash,format=raw,unit=0,file=$OvmfCode,readonly=on",
    "-drive", "if=pflash,format=raw,unit=1,file=$OvmfVars,snapshot=on",
    "-drive", "if=ide,index=0,media=disk,format=raw,file=$Image",
    "-drive", "if=ide,index=1,media=disk,format=raw,file=$LadybirdImage",
    "-netdev", "user,id=net0",
    "-device", "e1000,netdev=net0",
    "-serial", "stdio",
    "-boot", "order=c",
    "-no-reboot",
    "-no-shutdown"
)

if ($Accel -eq "tcg") {
    $Args += @("-accel", "tcg", "-cpu", "max")
}
else {
    $Args += @("-accel", "whpx,kernel-irqchip=off")
}

if ($Fullscreen) {
    $Args += "-full-screen"
}

& $QemuExe @Args
exit $LASTEXITCODE
