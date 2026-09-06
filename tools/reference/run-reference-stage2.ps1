param(
    [ValidateRange(512, 4096)]
    [int]$RamMiB = 1024,
    [ValidateRange(0, 8192)]
    [int]$MinWidth = 0,
    [ValidateRange(0, 4320)]
    [int]$MinHeight = 0
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot

function Fail([string]$Message) {
    Write-Host ""
    Write-Host "ERREUR: $Message" -ForegroundColor Red
    exit 1
}

if (($MinWidth -eq 0) -xor ($MinHeight -eq 0)) {
    Fail "MinWidth et MinHeight doivent etre tous deux nuls ou tous deux non nuls"
}

& ".\tools\reference\build-reference-stage2.ps1" `
    -MinWidth $MinWidth `
    -MinHeight $MinHeight
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$Image = Join-Path $RepoRoot "target\reference\bouchaud-reference-stage2.img"
if (-not (Test-Path -LiteralPath $Image -PathType Leaf)) {
    Fail "image Stage 2 introuvable: $Image"
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
Write-Host "=== Stage 2 / QEMU OVMF ===" -ForegroundColor Cyan
if ($MinWidth -gt 0) {
    Write-Host "Requete GOP: >= $MinWidth x $MinHeight (fallback firmware possible)" -ForegroundColor Yellow
}
Write-Host "Attendu: le bureau Bouchaud projete sur TOUT le GOP detecte au runtime." -ForegroundColor Yellow
Write-Host "Marqueurs:"
Write-Host "  BOUCHAUD_UEFI_ENTRY_OK"
Write-Host "  BOUCHAUD_UEFI_BOOTINFO_OK"
Write-Host "  BOUCHAUD_STAGE2_WM_RUNTIME_READY"
Write-Host "  BOUCHAUD_STAGE2_INPUT_READY"
Write-Host "  BOUCHAUD_STAGE2_WINDOW_MANAGER_READY"
Write-Host "Test: Demarrer puis double-clic Calculatrice/Fichiers/Rustpad."
Write-Host "Clic recu => BOUCHAUD_STAGE2_CLICK_DISPATCHED ..."
Write-Host ""

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
    "-net", "none",
    "-no-reboot",
    "-no-shutdown"
)

& $QemuExe @Args
exit $LASTEXITCODE
