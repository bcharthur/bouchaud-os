param(
    [ValidateRange(4096,16384)][int]$RamMiB=12288,
    [ValidateSet("tcg","whpx")][string]$Accel="tcg",
    [switch]$SkipBuild
)
$ErrorActionPreference="Stop"
$RepoRoot=Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot
function Fail([string]$Message){ Write-Host "ERREUR: $Message" -ForegroundColor Red; exit 1 }
if(-not $SkipBuild){ & ".\tools\reference\build-trigkey-stage2-usb.ps1"; if($LASTEXITCODE -ne 0){exit $LASTEXITCODE} }
$Image=Join-Path $RepoRoot "target\reference\bouchaud-trigkey-stage2-ladybird.img"
if(-not(Test-Path -LiteralPath $Image -PathType Leaf)){Fail "image TRIGKEY absente"}
$QemuCandidates=@("C:\Program Files\qemu\qemu-system-x86_64.exe","C:\Program Files (x86)\qemu\qemu-system-x86_64.exe")
$QemuExe=$QemuCandidates|Where-Object{Test-Path -LiteralPath $_ -PathType Leaf}|Select-Object -First 1
if(-not $QemuExe){$cmd=Get-Command qemu-system-x86_64 -ErrorAction SilentlyContinue;if($cmd){$QemuExe=$cmd.Source}}
if(-not $QemuExe){Fail "qemu-system-x86_64 introuvable"}
$Share=Join-Path (Split-Path -Parent $QemuExe) "share"
$Code=@((Join-Path $Share "edk2-x86_64-code.fd"),(Join-Path $Share "edk2-x86_64-code-secure.fd"))|Where-Object{Test-Path $_}|Select-Object -First 1
$Vars=@((Join-Path $Share "edk2-i386-vars.fd"),(Join-Path $Share "edk2-x86_64-vars.fd"))|Where-Object{Test-Path $_}|Select-Object -First 1
if(-not $Code -or -not $Vars){Fail "OVMF introuvable"}
Write-Host "=== TRIGKEY physical-layout regression / ONE DISK ===" -ForegroundColor Cyan
Write-Host "Aucun hdb Ladybird n'est attache. Si Ladybird demarre, le ramdisk UEFI est prouve." -ForegroundColor Yellow
Write-Host "Attendu: BOUCHAUD_TRIGKEY_LADYBIRD_RAMDISK_OK puis BOUCHAUD_STAGE2_LADYBIRD_RUNTIME_OK"
$args=@("-machine","pc","-m","$RamMiB","-smp","1","-drive","if=pflash,format=raw,unit=0,file=$Code,readonly=on","-drive","if=pflash,format=raw,unit=1,file=$Vars,snapshot=on","-drive","if=ide,index=0,media=disk,format=raw,file=$Image","-netdev","user,id=net0","-device","e1000,netdev=net0","-serial","stdio","-boot","order=c","-no-reboot","-no-shutdown")
if($Accel -eq "tcg"){$args+=@("-accel","tcg","-cpu","max")}else{$args+=@("-accel","whpx,kernel-irqchip=off")}
& $QemuExe @args
exit $LASTEXITCODE
