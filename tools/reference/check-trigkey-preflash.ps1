$ErrorActionPreference="Stop"
$RepoRoot=Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot
$Image=Join-Path $RepoRoot "target\reference\bouchaud-trigkey-stage2-ladybird.img"
if(-not(Test-Path -LiteralPath $Image -PathType Leaf)){ throw "Image absente: lance build-trigkey-stage2-usb.ps1" }
$h=Get-FileHash -Algorithm SHA256 -LiteralPath $Image
Write-Host "=== PRE-FLASH TRIGKEY ===" -ForegroundColor Cyan
Write-Host "Image : $($h.Path)"
Write-Host "SHA256: $($h.Hash)"
Write-Host ""
Write-Host "Regles de test physique:" -ForegroundColor Yellow
Write-Host "  - Secure Boot: disabled"
Write-Host "  - Boot Override: UEFI Lexar"
Write-Host "  - cable Ethernet branche pour le test RTL8168"
Write-Host "  - ne PAS selectionner le NVMe interne comme cible dans Rufus"
Write-Host "  - le chemin ramdisk Bouchaud ne lit/ecrit pas le NVMe pour Ladybird"
Write-Host ""
Write-Host "Marqueurs a rechercher via console/diagnostic futur:"
Write-Host "  BOUCHAUD_TRIGKEY_LADYBIRD_RAMDISK_OK"
Write-Host "  BOUCHAUD_TRIGKEY_RTL8168_DETECTED"
Write-Host "  BOUCHAUD_TRIGKEY_RTL8168_DRIVER_OK"
Write-Host "  BOUCHAUD_TRIGKEY_RTL8168_LINK_UP"
Write-Host "  BOUCHAUD_TRIGKEY_XHCI_PRESENT"
Write-Host "  BOUCHAUD_STAGE2_LADYBIRD_RUNTIME_OK"
Write-Host "BOUCHAUD_TRIGKEY_PREFLASH_CHECK_OK" -ForegroundColor Green
