# Depose dans le scenario les fichiers dont une machine INSTALLEE a besoin.
#
# POURQUOI L'ARCHIVE PORTE SON PROPRE CHARGEUR
# ============================================
#
# Le systeme vivant ne peut pas se recopier depuis la cle : il n'y a aucun
# pilote de stockage de masse USB, et le micrologiciel a rendu la main depuis
# longtemps. Ce que la machine a en main, c'est son ARCHIVE, chargee en memoire
# par le chargeur d'amorcage.
#
# On y depose donc, sous "install/", les trois fichiers que l'ESP du disque
# interne doit porter. L'installateur du noyau les y lit et les ecrit. C'est
# exactement ce que fait un installateur vivant qui transporte l'image du
# systeme qu'il pose -- la difference etant qu'ici l'image et le systeme vivant
# sont la meme chose.
#
# CE QUE CE SCRIPT NE DECIDE PAS
# ==============================
#
# Il ne decide pas quel fichier devient "bootx64.efi". C'est le shim de preboot
# quand il y en a un, et le chargeur sinon -- et cette regle est ecrite DANS
# l'outil d'image, en un seul endroit. Ecrire la meme regle ici garantirait
# qu'un jour les deux divergent, et la machine installee demarrerait sur autre
# chose que la cle.

param(
    [Parameter(Mandatory = $true)][string]$Scenario,
    [ValidateRange(0, 8192)][int]$MinWidth = 0,
    [ValidateRange(0, 4320)][int]$MinHeight = 0,
    [string]$PrebootShim = ""
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot
function Fail([string]$Message) { Write-Host ""; Write-Host "ERREUR: $Message" -ForegroundColor Red; exit 1 }

if (($MinWidth -eq 0) -xor ($MinHeight -eq 0)) { Fail "MinWidth et MinHeight doivent etre tous deux nuls ou tous deux non nuls" }
if (-not (Test-Path -LiteralPath $Scenario -PathType Container)) { Fail "scenario absent: $Scenario" }
if ($PrebootShim) {
    $PrebootShim = [System.IO.Path]::GetFullPath($PrebootShim)
    if (-not (Test-Path -LiteralPath $PrebootShim -PathType Leaf)) { Fail "preboot shim absent: $PrebootShim" }
}

Write-Host "=== Bouchaud OS - charge d'installation ===" -ForegroundColor Cyan

# Le MEME noyau que celui de l'image. Les deux constructions portent sur le
# meme arbre avec les memes options : la seconde est un no-op, et les octets
# sont identiques. Un noyau different dans l'archive et sur la cle donnerait
# une machine installee qui ne serait pas celle qu'on a essayee.
$UefiTarget = Join-Path $RepoRoot "targets\x86_64-bouchaud_os_uefi.json"
& cargo build --target "$UefiTarget" --no-default-features --features "uefi-boot,reference-bringup,reference-desktop"
if ($LASTEXITCODE -ne 0) { Fail "build noyau en echec" }

$KernelCandidates = @(
    (Join-Path $RepoRoot "target\x86_64-bouchaud_os_uefi\debug\bouchaud-os"),
    (Join-Path $RepoRoot "target\x86_64-bouchaud_os_uefi\debug\bouchaud-os.exe")
)
$Kernel = $KernelCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
if (-not $Kernel) { Fail "ELF noyau introuvable" }

$Install = Join-Path $Scenario "install"
if (Test-Path -LiteralPath $Install) { Remove-Item -LiteralPath $Install -Recurse -Force }
New-Item -ItemType Directory -Path $Install -Force | Out-Null

$BuilderSource = Join-Path $RepoRoot "tools\reference\uefi-image-builder"
$BuilderTemp = Join-Path $env:TEMP "bouchaud-install-payload-builder"
if (Test-Path -LiteralPath $BuilderTemp) { Remove-Item -Recurse -Force $BuilderTemp }
Copy-Item -Recurse -Force $BuilderSource $BuilderTemp
$BuilderTarget = Join-Path $RepoRoot "target\reference\image-builder-target"

Push-Location $BuilderTemp
try {
    if ($MinWidth -gt 0 -and $PrebootShim) {
        & cargo +nightly-2026-06-01 run --release --target-dir "$BuilderTarget" -- --charge-installation "$Kernel" "$Install" "$MinWidth" "$MinHeight" "$PrebootShim"
    }
    elseif ($MinWidth -gt 0) {
        & cargo +nightly-2026-06-01 run --release --target-dir "$BuilderTarget" -- --charge-installation "$Kernel" "$Install" "$MinWidth" "$MinHeight"
    }
    elseif ($PrebootShim) {
        & cargo +nightly-2026-06-01 run --release --target-dir "$BuilderTarget" -- --charge-installation "$Kernel" "$Install" "$PrebootShim"
    }
    else {
        & cargo +nightly-2026-06-01 run --release --target-dir "$BuilderTarget" -- --charge-installation "$Kernel" "$Install"
    }
    $Code = $LASTEXITCODE
}
finally {
    Pop-Location
}
if ($Code -ne 0) { Fail "emission de la charge d'installation en echec" }

# Les trois fichiers sont OBLIGATOIRES. Sans le chargeur, l'ESP posee sur le
# disque serait un systeme de fichiers valide que le micrologiciel ignorerait :
# une installation qui se declare reussie et une machine qui ne demarre pas.
foreach ($Requis in @("bootx64.efi", "kernel-x86_64", "boot.json")) {
    $Chemin = Join-Path $Install $Requis
    if (-not (Test-Path -LiteralPath $Chemin -PathType Leaf)) { Fail "charge d'installation incomplete: $Requis" }
}

$Total = (Get-ChildItem -LiteralPath $Install -File | Measure-Object -Property Length -Sum).Sum
Write-Host "BOUCHAUD_INSTALL_PAYLOAD_STAGED dir=$Install bytes=$Total" -ForegroundColor Green
