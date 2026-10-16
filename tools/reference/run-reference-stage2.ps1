# BOUCHAUD_QEMU_WINDOWS_SAFE_V1 -- classic Stage 2, optional simulated Trigkey.
param(
    [ValidateRange(512,16384)][int]$RamMiB = 1024,
    [ValidateRange(0,8192)][int]$MinWidth = 0,
    [ValidateRange(0,4320)][int]$MinHeight = 0,
    [switch]$CommeTrigkey,
    [ValidateRange(0,64)][int]$Coeurs = 0,
    [switch]$SkipBuild,
    [ValidateSet('usb','ps2','none')][string]$InputMode = 'usb',
    [ValidateSet('mouse','tablet')][string]$Pointer = 'mouse',
    [ValidateSet('auto','gtk','sdl','none')][string]$Display = 'auto',
    [switch]$Fullscreen,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot
. (Join-Path $PSScriptRoot 'qemu-windows-common.ps1')

if (($MinWidth -eq 0) -xor ($MinHeight -eq 0)) {
    throw 'MinWidth et MinHeight doivent etre simultanement nuls ou non nuls.'
}
if ($DryRun -and -not $SkipBuild) { throw '-DryRun requiert -SkipBuild.' }
if (-not $SkipBuild) {
    & (Join-Path $PSScriptRoot 'build-reference-stage2.ps1') -MinWidth $MinWidth -MinHeight $MinHeight
    if ($LASTEXITCODE -ne 0) { throw "Build Stage 2 echoue : $LASTEXITCODE" }
}
$Image = Join-Path $RepoRoot 'target\reference\bouchaud-reference-stage2.img'
if (-not (Test-Path -LiteralPath $Image -PathType Leaf)) { throw "Image Stage 2 absente : $Image" }
if ($CommeTrigkey) {
    if ($Coeurs -eq 0) { $Coeurs = 16 }
    if ($RamMiB -lt 4096) { $RamMiB = 4096 }
} elseif ($Coeurs -eq 0) { $Coeurs = 1 }
$QemuExe = Get-BouchaudQemuExe
$Ovmf = Get-BouchaudOvmf -QemuExe $QemuExe
$Serial = New-BouchaudQemuSerialPath -RepoRoot $RepoRoot -Profile 'stage2' -DryRun:$DryRun
$ArgsQemu = @(
    '-machine','q35', '-m',"$RamMiB", '-smp',"$Coeurs",
    '-accel','tcg', '-cpu','max',
    '-drive',"if=pflash,format=raw,unit=0,file=$($Ovmf.Code),readonly=on",
    '-drive',"if=pflash,format=raw,unit=1,file=$($Ovmf.Vars),snapshot=on",
    '-drive',"format=raw,file=$Image,snapshot=on",
    '-vga','std', '-serial',"file:$Serial", '-monitor','stdio',
    '-no-reboot', '-no-shutdown'
)
if ($CommeTrigkey) {
    # Only this explicit mode attaches a *virtual* NVMe. Never a physical drive.
    $Nvme = Join-Path $RepoRoot 'target\reference\trigkey-nvme.img'
    if (-not (Test-Path -LiteralPath $Nvme -PathType Leaf)) {
        if ($DryRun) {
            Write-Host "QEMU_DRY_RUN_NVME_WOULD_CREATE=$Nvme"
        } else {
            $QemuImg = Join-Path (Split-Path -Parent $QemuExe) 'qemu-img.exe'
            if (-not (Test-Path -LiteralPath $QemuImg -PathType Leaf)) {
                throw "qemu-img.exe introuvable : $QemuImg"
            }
            & $QemuImg create -f raw $Nvme 256M | Out-Null
            if ($LASTEXITCODE -ne 0) { throw 'Creation NVMe virtuel echouee.' }
        }
    }
    $ArgsQemu += @(
        '-drive',"id=nv,file=$Nvme,format=raw,if=none,snapshot=on",
        '-device','nvme,drive=nv,serial=BOUCHAUDTRIGKEY',
        '-netdev','user,id=net0', '-device','e1000,netdev=net0'
    )
    Write-Host 'QEMU_PROFIL=COMME_TRIGKEY (hardware emule, PAS validation physique)'
} else { $ArgsQemu += @('-net','none') }
$ArgsQemu += @(Get-BouchaudInputArguments -InputMode $InputMode -Pointer $Pointer)
$ArgsQemu += @(Get-BouchaudDisplayArguments -QemuExe $QemuExe -Display $Display -Fullscreen:$Fullscreen)
Write-Host '=== BOUCHAUD STAGE2 / QEMU WINDOWS ===' -ForegroundColor Cyan
Write-Host "BOOT_IMAGE=$Image COEURS=$Coeurs INPUT=$InputMode POINTER=$Pointer"
$rc = Invoke-BouchaudQemu -QemuExe $QemuExe -Arguments $ArgsQemu -SerialPath $Serial -DryRun:$DryRun
if ($DryRun) { return }
Write-Host "QEMU_EXIT_CODE=$rc"
Write-Host "QEMU_SERIAL=$Serial"
exit $rc
