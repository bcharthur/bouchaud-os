param(
    [string]$Output = "",
    [switch]$ForceLadybird
)
$ErrorActionPreference="Stop"
$RepoRoot=Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot
function Fail([string]$Message){ Write-Host "ERREUR: $Message" -ForegroundColor Red; exit 1 }

Write-Host "=== Bouchaud OS - TRIGKEY Stage 2 single-USB image ===" -ForegroundColor Cyan
& ".\tools\reference\prepare-reference-ladybird.ps1" -Force:$ForceLadybird
if($LASTEXITCODE -ne 0){ exit $LASTEXITCODE }
$Ramdisk=Join-Path $RepoRoot "ladybird-browser.img"
if(-not (Test-Path -LiteralPath $Ramdisk -PathType Leaf)){ Fail "ladybird-browser.img absent" }
& python ".\tools\reference\verify-reference-ladybird-image.py" $Ramdisk
if($LASTEXITCODE -ne 0){ Fail "ladybird-browser.img invalide" }
if(-not $Output){ $Output=Join-Path $RepoRoot "target\reference\bouchaud-trigkey-stage2-ladybird.img" }
& ".\tools\reference\build-reference-stage2.ps1" -Output $Output -Ramdisk $Ramdisk
if($LASTEXITCODE -ne 0){ exit $LASTEXITCODE }
$Output=[System.IO.Path]::GetFullPath($Output)
$Bytes=(Get-Item -LiteralPath $Output).Length
$Hash=(Get-FileHash -Algorithm SHA256 -LiteralPath $Output).Hash
Write-Host ""
Write-Host "TRIGKEY_USB_IMAGE=$Output" -ForegroundColor Green
Write-Host "TRIGKEY_USB_BYTES=$Bytes"
Write-Host "TRIGKEY_USB_SHA256=$Hash"
Write-Host "BOUCHAUD_TRIGKEY_STAGE2_USB_IMAGE_READY" -ForegroundColor Green
Write-Host "Flash: Rufus -> Selection de demarrage -> cette image -> mode DD si propose." -ForegroundColor Yellow
