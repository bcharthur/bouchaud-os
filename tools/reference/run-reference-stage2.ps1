param(
    [ValidateRange(512, 16384)]
    [int]$RamMiB = 1024,
    [ValidateRange(0, 8192)]
    [int]$MinWidth = 0,
    [ValidateRange(0, 4320)]
    [int]$MinHeight = 0,

    # CE QUE QEMU NE TESTAIT PAS, ET QUI EST PRECISEMENT CE QUI CASSE.
    #
    # Ce script tournait en `-smp 1`, sans xHCI et sans NVMe. Sur cette forme :
    #
    #   SMP4_AP_STARTED count=0 reason=single-vcpu
    #   BOUCHAUD_HWPROBE_XHCI absent
    #   BOUCHAUD_NVME_ABSENT
    #
    # Autrement dit, ni la frontiere SMP -- celle qui a double-faute sur la
    # machine de reference --, ni le chemin USB HID -- celui du clavier muet --,
    # ni le disque. QEMU validait une image qui ne partageait presque rien avec
    # ce qui allait tourner.
    #
    # `-CommeTrigkey` donne a QEMU la FORME de la machine de reference : seize
    # coeurs, un controleur xHCI avec clavier et souris USB, un NVMe et une
    # carte reseau. Verifie : `SMP4_AP_STARTED count=15 expected=15`,
    # `BOUCHAUD_STAGE2_ENTREE_DECIDEE claviers_usb=1 ps2_clavier=0`,
    # `BOUCHAUD_NVME_GREEN`, et les deux marqueurs de frontiere dans l'ordre
    # avec le meme `rsp=0x18000014d50` que le releve physique.
    #
    # CE QUE CE PROFIL NE REPRODUIT PAS, et il faut le dire : le processeur
    # reste emule, le controleur xHCI est celui de QEMU et non l'AMD de la
    # TRIGKEY, et le clavier USB de QEMU n'est pas un recepteur Logitech
    # unifie. Une forme proche n'est pas la meme machine.
    [switch]$CommeTrigkey,

    [ValidateRange(1, 64)]
    [int]$Coeurs = 0
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

if ($CommeTrigkey) {
    if ($Coeurs -eq 0) { $Coeurs = 16 }
    if ($RamMiB -lt 4096) { $RamMiB = 4096 }
}
elseif ($Coeurs -eq 0) {
    $Coeurs = 1
}

$Args = @(
    "-machine", "q35",
    "-m", "$RamMiB",
    "-smp", "$Coeurs",
    "-accel", "tcg",
    "-cpu", "max",
    "-drive", "if=pflash,format=raw,unit=0,file=$OvmfCode,readonly=on",
    "-drive", "if=pflash,format=raw,unit=1,file=$OvmfVars,snapshot=on",
    "-drive", "format=raw,file=$Image",
    "-serial", "stdio",
    "-no-reboot",
    "-no-shutdown"
)

if ($CommeTrigkey) {
    $Nvme = Join-Path $RepoRoot "target\reference\trigkey-nvme.img"
    if (-not (Test-Path -LiteralPath $Nvme -PathType Leaf)) {
        $Qemu = Split-Path -Parent $QemuExe
        $QemuImg = Join-Path $Qemu "qemu-img.exe"
        if (-not (Test-Path -LiteralPath $QemuImg -PathType Leaf)) {
            Fail "qemu-img introuvable a cote de qemu-system-x86_64 : $QemuImg"
        }
        & $QemuImg create -f raw $Nvme 256M | Out-Null
        if ($LASTEXITCODE -ne 0) { Fail "creation du disque NVMe en echec" }
    }
    $Args += @(
        "-drive", "id=nv,file=$Nvme,format=raw,if=none",
        "-device", "nvme,drive=nv,serial=BOUCHAUDTRIGKEY",
        "-device", "qemu-xhci,id=xhci",
        "-device", "usb-kbd,bus=xhci.0",
        "-device", "usb-mouse,bus=xhci.0",
        "-netdev", "user,id=net0",
        "-device", "e1000,netdev=net0"
    )

    Write-Host ""
    Write-Host "--- FORME TRIGKEY : ce qui devient testable ---" -ForegroundColor Cyan
    @(
        "SMP4_AP_STARTED count=15 expected=15    les quinze AP demarrent",
        "SMP_HANDOFF_BEFORE_STI ... cpus_en_ligne=16",
        "SMP_HANDOFF_AFTER_FIRST_IRQ vector=0x20 les deux, DANS CET ORDRE",
        "BOUCHAUD_STAGE2_ENTREE_DECIDEE claviers_usb=1 ps2_clavier=0",
        "BOUCHAUD_NVME_GREEN                     le disque repond",
        "[USB-HID-POINT] genre=clavier evenements= doit MONTER quand on tape",
        "",
        "Ce profil ne reproduit PAS la machine : processeur emule, xHCI de",
        "QEMU et non l'AMD de la TRIGKEY, clavier USB simple et non un",
        "recepteur Logitech unifie. Une forme proche n'est pas la meme machine."
    ) | ForEach-Object { Write-Host "  $_" }
    Write-Host ""
}
else {
    $Args += @("-net", "none")
}

& $QemuExe @Args
exit $LASTEXITCODE
