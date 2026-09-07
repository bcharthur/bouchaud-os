param(
    [ValidateRange(512, 4096)]
    [int]$RamMiB = 1024
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot

function Fail([string]$Message) {
    Write-Host ""
    Write-Host "ERREUR: $Message" -ForegroundColor Red
    exit 1
}

& ".\tools\reference\build-reference-uefi.ps1"
if ($LASTEXITCODE -ne 0) {
    exit $LASTEXITCODE
}

$Image = Join-Path $RepoRoot "target\reference\bouchaud-reference-uefi-stage1.img"
if (-not (Test-Path -LiteralPath $Image -PathType Leaf)) {
    Fail "image UEFI introuvable: $Image"
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
    if ($QemuCmd) {
        $QemuExe = $QemuCmd.Source
    }
}
if (-not $QemuExe) {
    Fail "qemu-system-x86_64 introuvable"
}

$QemuRoot = Split-Path -Parent $QemuExe
$Share = Join-Path $QemuRoot "share"

$CodeCandidates = @(
    (Join-Path $Share "edk2-x86_64-code.fd"),
    (Join-Path $Share "edk2-x86_64-code-secure.fd")
)
$OvmfCode = $CodeCandidates | Where-Object {
    Test-Path -LiteralPath $_ -PathType Leaf
} | Select-Object -First 1

if (-not $OvmfCode -and (Test-Path -LiteralPath $Share)) {
    $OvmfCode = Get-ChildItem -LiteralPath $Share -File -Recurse `
        | Where-Object { $_.Name -match 'x86_64.*code.*\.fd$' } `
        | Select-Object -First 1 -ExpandProperty FullName
}

$VarsCandidates = @(
    (Join-Path $Share "edk2-i386-vars.fd"),
    (Join-Path $Share "edk2-x86_64-vars.fd")
)
$OvmfVars = $VarsCandidates | Where-Object {
    Test-Path -LiteralPath $_ -PathType Leaf
} | Select-Object -First 1

if (-not $OvmfVars -and (Test-Path -LiteralPath $Share)) {
    $OvmfVars = Get-ChildItem -LiteralPath $Share -File -Recurse `
        | Where-Object { $_.Name -match 'vars.*\.fd$' } `
        | Select-Object -First 1 -ExpandProperty FullName
}

if (-not $OvmfCode) {
    Fail "firmware OVMF/edk2 CODE introuvable dans $Share"
}
if (-not $OvmfVars) {
    Fail "firmware OVMF/edk2 VARS introuvable dans $Share"
}

Write-Host ""
Write-Host "=== Bouchaud UEFI / OVMF runtime proof ===" -ForegroundColor Cyan
Write-Host "QEMU : $QemuExe"
Write-Host "CODE : $OvmfCode"
Write-Host "VARS : $OvmfVars"
Write-Host "IMG  : $Image"
Write-Host ""
Write-Host "Marqueurs attendus:" -ForegroundColor Cyan
Write-Host "  BOUCHAUD_UEFI_ENTRY_OK"
Write-Host "  [BRINGUP] mode=reference-stage1 firmware=uefi ..."
Write-Host "  [BRINGUP] uefi framebuffer=..."
Write-Host "  BOUCHAUD_UEFI_BOOTINFO_OK"
Write-Host ""
Write-Host "Mode regression serie/headless: le noyau Stage 1 dessine bien le GOP, mais -display none masque volontairement la fenetre." -ForegroundColor Yellow

$Args = @(
    "-machine", "q35",
    "-m", "$RamMiB",
    "-smp", "1",
    "-accel", "tcg",
    "-cpu", "max",
    "-drive", "if=pflash,format=raw,unit=0,file=$OvmfCode,readonly=on",
    "-drive", "if=pflash,format=raw,unit=1,file=$OvmfVars,snapshot=on",
    "-drive", "format=raw,file=$Image",
    "-serial", "stdio",
    "-display", "none",
    "-net", "none",
    "-no-reboot",
    "-no-shutdown"
)

& $QemuExe @Args
exit $LASTEXITCODE
