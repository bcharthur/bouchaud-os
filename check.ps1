# Verification de compilation (sans QEMU) : compile le noyau pour la cible
# custom (build-std) et conserve la fenetre ouverte pour lire les erreurs.
Set-Location $PSScriptRoot
$ErrorActionPreference = 'Continue'
Write-Host "=== cargo +nightly build (cible x86_64-bouchaud_os) ===" -ForegroundColor Cyan
cargo +nightly build 2>&1 | Tee-Object -FilePath build.log
Write-Host ""
Write-Host "=== EXIT CODE = $LASTEXITCODE ===" -ForegroundColor Yellow
Read-Host "Compilation terminee. Appuie sur Entree pour fermer"
