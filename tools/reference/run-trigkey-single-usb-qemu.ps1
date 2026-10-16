# BOUCHAUD_QEMU_WINDOWS_SAFE_V1 -- Product M1 / UEFI single USB image.
param(
    [ValidateRange(4096,16384)][int]$RamMiB = 12288,
    [ValidateSet('tcg','whpx')][string]$Accel = 'tcg',
    [switch]$SkipBuild,
    [ValidateSet('usb','ps2','none')][string]$InputMode = 'usb',
    [ValidateSet('mouse','tablet')][string]$Pointer = 'mouse',
    [ValidateSet('auto','gtk','sdl','none')][string]$Display = 'auto',
    [switch]$Fullscreen,
    [switch]$DryRun,
    # Opt-in two-disk test; NEVER used in the single-USB physical regression.
    [switch]$PersistLab,
    # Optional QEMU shutdown contract; NOT a proof of ACPI shutdown.
    [switch]$DebugExit,
    [switch]$ExitOnShutdown
)
$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot
. (Join-Path $PSScriptRoot 'qemu-windows-common.ps1')

if ($DryRun -and -not $SkipBuild) {
    throw '-DryRun refuse la compilation : utilise -DryRun -SkipBuild.'
}
if (-not $SkipBuild) {
    & (Join-Path $PSScriptRoot 'build-trigkey-stage2-usb.ps1')
    if ($LASTEXITCODE -ne 0) { throw "Build Trigkey echoue : $LASTEXITCODE" }
}
$Image = Join-Path $RepoRoot 'target\reference\bouchaud-trigkey-stage2-ladybird.img'
if (-not (Test-Path -LiteralPath $Image -PathType Leaf)) { throw "Image Trigkey absente : $Image" }

$QemuExe = Get-BouchaudQemuExe
$Ovmf = Get-BouchaudOvmf -QemuExe $QemuExe
$Serial = New-BouchaudQemuSerialPath -RepoRoot $RepoRoot -Profile 'trigkey-m1' -DryRun:$DryRun
$ArgsQemu = @(
    '-machine','pc',
    '-m',"$RamMiB",
    '-smp','1',
    '-drive',"if=pflash,format=raw,unit=0,file=$($Ovmf.Code),readonly=on",
    '-drive',"if=pflash,format=raw,unit=1,file=$($Ovmf.Vars),snapshot=on",
    '-drive',"if=ide,index=0,media=disk,format=raw,file=$Image,snapshot=on",
    '-netdev','user,id=net0',
    '-device','e1000,netdev=net0',
    '-vga','std',
    '-serial',"file:$Serial",
    '-monitor','stdio',
    '-boot','order=c',
    '-no-reboot',
    '-no-shutdown'
)
if ($PersistLab) {
    $DataImage = Join-Path $RepoRoot 'target\reference\qemu-data\m1-1-persist-lab.img'
    if (-not (Test-Path -LiteralPath $DataImage -PathType Leaf)) {
        throw "Image persistante manquante : $DataImage. Executer: py -3 tools/product/product-m1-1/create_persist_lab.py --create --repo ."
    }
    $ExpectedBytes = (128 * 1024 * 1024) + (1026 * 512)
    if ((Get-Item -LiteralPath $DataImage).Length -ne $ExpectedBytes) {
        throw "Taille data-image inattendue ; refus pour eviter toute ecriture sur une autre cible."
    }
    # Explicit writeable SECOND disk, on a named test file only.
    $ArgsQemu += @('-drive',"if=ide,index=1,media=disk,format=raw,file=$DataImage")
    Write-Host "QEMU_PERSIST_LAB=ON: SECOND_DISK_WRITE_ENABLED=$DataImage" -ForegroundColor Yellow
    Write-Host 'Les changements de ce disque persistent. Image systeme toujours en snapshot.' -ForegroundColor Yellow
} else {
    Write-Host 'QEMU_PERSIST_LAB=OFF: ONE_DISK_STAGE2_PROOF' -ForegroundColor Green
}
if ($DebugExit) { $ArgsQemu += @('-device','isa-debug-exit,iobase=0xf4,iosize=0x04') }
$ArgsQemu += @(Get-BouchaudInputArguments -InputMode $InputMode -Pointer $Pointer)
$ArgsQemu += @(Get-BouchaudDisplayArguments -QemuExe $QemuExe -Display $Display -Fullscreen:$Fullscreen)
if ($Accel -eq 'tcg') { $ArgsQemu += @('-accel','tcg','-cpu','max') }
else { $ArgsQemu += @('-accel','whpx,kernel-irqchip=off') }

Write-Host '=== BOUCHAUD PRODUCT M1 / UEFI SINGLE USB / QEMU WINDOWS ===' -ForegroundColor Cyan
Write-Host "BOOT_IMAGE=$Image"
Write-Host "INPUT=$InputMode POINTER=$Pointer ACCEL=$Accel"
Write-Host $(if ($PersistLab) { 'BOOT_DEVICE=TWO_DISKS_PERSISTENCE_LAB' } else { 'BOOT_DEVICE=ONE_DISK_RAMDISK_PROOF' })
Write-Host 'ATTENDU: BOUCHAUD_TRIGKEY_LADYBIRD_RAMDISK_OK / BOUCHAUD_STAGE2_LADYBIRD_RUNTIME_OK'
Write-Host 'Release pointer from QEMU: Ctrl+Alt+G'
# BOUCHAUD_M12_SHUTDOWN_RUNNER_HOTFIX_V1
# Default preserved: -no-shutdown remains for all normal runs.
# With -ExitOnShutdown, QEMU can terminate when the guest powers off by ACPI.
# This option is incompatible with isa-debug-exit; each proves a separate path.
if ($ExitOnShutdown -and $DebugExit) { throw 'QEMU_ACPI_DEBUG_EXIT_CONFLICT' }
if ($ExitOnShutdown) {
    if ($ArgsQemu -notcontains '-no-shutdown') { throw 'QEMU_NO_SHUTDOWN_FLAG_MISSING' }
    $ArgsQemu = @($ArgsQemu | Where-Object { $_ -ne '-no-shutdown' })
    Write-Host 'QEMU_EXIT_ON_GUEST_SHUTDOWN=ON ACPI_OBSERVATION_MODE=ON'
}
$rc = Invoke-BouchaudQemu -QemuExe $QemuExe -Arguments $ArgsQemu -SerialPath $Serial -DryRun:$DryRun
if ($DryRun) { return }
Write-Host "QEMU_EXIT_CODE=$rc"
Write-Host "QEMU_SERIAL=$Serial"
# The kernel writes EXIT_OK=0x10 to 0xf4; QEMU returns (0x10 << 1)|1 = 33.
if ($DebugExit -and $rc -eq 33) { Write-Host 'BOUCHAUD_QEMU_GUEST_SHUTDOWN_OK' -ForegroundColor Green; exit 0 }
exit $rc
