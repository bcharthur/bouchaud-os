# BOUCHAUD_QEMU_WINDOWS_SAFE_V1 -- two disk legacy Stage 2 Ladybird test.
#
# BOUCHAUD_PERF_SESSION_V1 -- A/B de performance, une variable a la fois :
#   -Smp 1|2|4       coeurs virtuels ; 1 reste la REFERENCE. 2 et 4 ne sont pas
#                    supposes fonctionner : le resume de fin dit combien de
#                    coeurs l'ordonnanceur a REELLEMENT mis en ligne.
#   -Mimalloc actuelle|delai10|reference
#                    purge de mimalloc du navigateur : actuelle = profil
#                    produit (delai 0) ; delai10 = defaut de mimalloc ;
#                    reference = aucune purge (borne du gain). Hors `actuelle`,
#                    une COPIE du disque Ladybird est derivee ; l'original
#                    n'est jamais ouvert en ecriture.
#   -Irqchip off|on  WHPX : APIC emule par QEMU en espace utilisateur (off,
#                    reference) ou par Hyper-V (on). Avec `off`, chaque
#                    programmation du minuteur APIC, chaque EOI et chaque IPI
#                    est une sortie de VM traitee par QEMU.
# A la fermeture de QEMU, tools\perf\analyse_session.py resume la session :
# horloge (et sa coherence), coeurs en ligne, CPU et fautes par processus,
# appels systeme, composition, pauses.
param(
    [ValidateRange(4096,16384)][int]$RamMiB = 12288,
    [ValidateSet(1,2,4)][int]$Smp = 1,
    [ValidateSet('actuelle','delai10','reference')][string]$Mimalloc = 'actuelle',
    [ValidateSet('off','on')][string]$Irqchip = 'off',
    [ValidateSet('tcg','whpx')][string]$Accel = 'tcg',
    [switch]$Fullscreen,
    [switch]$SkipPrepare,
    [switch]$SkipBuild,
    [ValidateSet('usb','ps2','none')][string]$InputMode = 'usb',
    [ValidateSet('mouse','tablet')][string]$Pointer = 'mouse',
    [ValidateSet('auto','gtk','sdl','none')][string]$Display = 'auto',
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot
. (Join-Path $PSScriptRoot 'qemu-windows-common.ps1')

if ($DryRun -and (-not $SkipBuild -or -not $SkipPrepare)) {
    throw '-DryRun requiert -SkipBuild -SkipPrepare : aucune fabrication autorisee.'
}
if (-not $SkipPrepare) {
    & (Join-Path $PSScriptRoot 'prepare-reference-ladybird.ps1')
    if ($LASTEXITCODE -ne 0) { throw "Preparation Ladybird echouee : $LASTEXITCODE" }
}
if (-not $SkipBuild) {
    & (Join-Path $PSScriptRoot 'build-reference-stage2.ps1')
    if ($LASTEXITCODE -ne 0) { throw "Build Stage 2 echoue : $LASTEXITCODE" }
}
$Image = Join-Path $RepoRoot 'target\reference\bouchaud-reference-stage2.img'
$LadybirdImage = Join-Path $RepoRoot 'ladybird-browser.img'
foreach ($p in @($Image,$LadybirdImage)) {
    if (-not (Test-Path -LiteralPath $p -PathType Leaf)) { throw "Image absente : $p" }
}
if (-not $DryRun) {
    & python (Join-Path $PSScriptRoot 'verify-reference-ladybird-image.py') $LadybirdImage
    if ($LASTEXITCODE -ne 0) { throw 'Image Ladybird invalide.' }
}
if ($Mimalloc -ne 'actuelle') {
    $Derivee = Join-Path $RepoRoot "target\reference\ladybird-browser-$Mimalloc.img"
    if (-not $DryRun) {
        & python -I (Join-Path $RepoRoot 'tools\e2e\ladybird\prepare_disque.py') --image $LadybirdImage `
            --conserve-session --mimalloc $Mimalloc --sortie $Derivee
        if ($LASTEXITCODE -ne 0) { throw "Derivation du disque ($Mimalloc) echouee : $LASTEXITCODE" }
    }
    $LadybirdImage = $Derivee
}
$QemuExe = Get-BouchaudQemuExe
$Ovmf = Get-BouchaudOvmf -QemuExe $QemuExe
$Serial = New-BouchaudQemuSerialPath -RepoRoot $RepoRoot -Profile 'stage2-ladybird' -DryRun:$DryRun
$ArgsQemu = @(
    '-machine','pc', '-m',"$RamMiB", '-smp',"$Smp",
    '-drive',"if=pflash,format=raw,unit=0,file=$($Ovmf.Code),readonly=on",
    '-drive',"if=pflash,format=raw,unit=1,file=$($Ovmf.Vars),snapshot=on",
    '-drive',"if=ide,index=0,media=disk,format=raw,file=$Image,snapshot=on",
    '-drive',"if=ide,index=1,media=disk,format=raw,file=$LadybirdImage,snapshot=on",
    '-netdev','user,id=net0', '-device','e1000,netdev=net0',
    '-vga','std', '-serial',"file:$Serial", '-monitor','stdio',
    '-boot','order=c', '-no-reboot', '-no-shutdown'
)
$ArgsQemu += @(Get-BouchaudInputArguments -InputMode $InputMode -Pointer $Pointer)
$ArgsQemu += @(Get-BouchaudDisplayArguments -QemuExe $QemuExe -Display $Display -Fullscreen:$Fullscreen)
if ($Accel -eq 'tcg') { $ArgsQemu += @('-accel','tcg','-cpu','max') }
else { $ArgsQemu += @('-accel',"whpx,kernel-irqchip=$Irqchip") }
Write-Host '=== BOUCHAUD STAGE2 / LADYBIRD / TWO-DISK LEGACY ===' -ForegroundColor Cyan
Write-Host 'NOTE: ce profil n est PAS le test single-USB de Product M1.' -ForegroundColor Yellow
Write-Host "BOOT_IMAGE=$Image LADYBIRD_IMAGE=$LadybirdImage"
Write-Host "INPUT=$InputMode POINTER=$Pointer ACCEL=$Accel SMP=$Smp MIMALLOC=$Mimalloc IRQCHIP=$Irqchip"
$Debut = Get-Date
$rc = Invoke-BouchaudQemu -QemuExe $QemuExe -Arguments $ArgsQemu -SerialPath $Serial -DryRun:$DryRun
$DureeHoteS = [math]::Round(((Get-Date) - $Debut).TotalSeconds, 1)
if ($DryRun) { return }
Write-Host "QEMU_EXIT_CODE=$rc"
Write-Host "QEMU_SERIAL=$Serial"
if (Test-Path -LiteralPath $Serial) {
    $Resume = [System.IO.Path]::ChangeExtension($Serial, '.perf.json')
    Write-Host "=== RESUME DE SESSION (SMP=$Smp MIMALLOC=$Mimalloc ACCEL=$Accel IRQCHIP=$Irqchip, $DureeHoteS s hote) ===" -ForegroundColor Cyan
    & python (Join-Path $RepoRoot 'tools\perf\analyse_session.py') $Serial --json $Resume --duree-hote-s "$DureeHoteS"
    Write-Host "PERF_JSON=$Resume"
}
exit $rc
