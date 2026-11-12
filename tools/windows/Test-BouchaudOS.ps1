<#
.SYNOPSIS
  BOUCHAUD_WINDOWS_V1 -- teste l'image de livraison, ou ouvre son BUREAU sous QEMU depuis Windows.

.DESCRIPTION
  -Mode Livraison (defaut) : tools/release/test_livraison_qemu.py sous Linux
    (WSL ; QEMU et OVMF installes DANS WSL : apt install qemu-system-x86 ovmf).
    L'image exacte demarree, Terminal/DOOM/Ladybird ouverts a la souris,
    premieres trames exigees. -ExigeLadybird : une image sans Ladybird echoue.
  -Mode Bureau : QEMU POUR WINDOWS, fenetre interactive : le bureau Atelier de
    l'image de livraison, avec souris et clavier, sur une COPIE jetable du
    disque (l'image n'est jamais modifiee). Disque NVMe, carte e1000, OVMF de
    l'installation QEMU (tools/reference/qemu-windows-common.ps1).
    -DryRun : affiche la commande QEMU sans la lancer.

  NON QUALIFIE SUR WINDOWS par la session qui l'a ecrit : aucune machine
  Windows n'etait disponible. La CI execute ces scripts sous pwsh (Linux).

.EXAMPLE
  .\tools\windows\Test-BouchaudOS.ps1 -Mode Bureau
  .\tools\windows\Test-BouchaudOS.ps1 -Mode Livraison -ExigeLadybird
#>
[CmdletBinding()]
param(
    [ValidateSet('Livraison', 'Bureau')][string]$Mode = 'Livraison',
    [string]$Image = 'target/livraison',
    [switch]$ExigeLadybird,
    [ValidateSet('auto', 'gtk', 'sdl')][string]$Affichage = 'auto',
    [int]$MemoireMo = 4096,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'BouchaudWindows.psm1') -Force
$journal = Start-JournalBouchaud "test-$($Mode.ToLowerInvariant())"
try {
    $racine = Get-RacineDepot
    $dossier = Join-Path $racine $Image
    $disque = Join-Path $dossier 'disque-ab.img'
    if (-not (Test-Path -LiteralPath $disque)) { Stop-Bouchaud "$disque absent : lancer Build-BouchaudOS.ps1" }
    $empreinte = Assert-Empreinte $disque
    $c = Get-Composants $dossier
    Write-Ok "image sha256=$empreinte ladybird=$($c['ladybird']) livrable=$($c['livrable'])"

    if ($Mode -eq 'Livraison') {
        $exige = ''
        if ($ExigeLadybird) { $exige = ' --exige-ladybird' }
        $code = Invoke-Linux ("python3 tools/release/test_livraison_qemu.py '{0}'{1}" -f (ConvertTo-CheminLinux $dossier), $exige)
        if ($code -ne 0) { Stop-Bouchaud "test de livraison en echec (code $code) ; preuves : $Image\livraison-evidence.json" }
        Write-Host "BOUCHAUD_TEST_WINDOWS_OK mode=livraison sha256=$empreinte journal=$journal"
        return
    }

    # --- Bureau interactif, QEMU pour Windows (ou Linux) ---------------------------
    . (Join-Path $racine 'tools/reference/qemu-windows-common.ps1')
    $qemu = Get-BouchaudQemuExe
    $ovmf = Get-BouchaudOvmf -QemuExe $qemu
    $jetable = Join-Path ([System.IO.Path]::GetTempPath()) ("bouchaud-bureau-{0}" -f $PID)
    New-Item -ItemType Directory -Force -Path $jetable | Out-Null
    $copie = Join-Path $jetable 'disque.img'
    $vars = Join-Path $jetable 'vars.fd'
    if (-not $DryRun) {
        Copy-Item -LiteralPath $disque -Destination $copie
        Copy-Item -LiteralPath $ovmf.Vars -Destination $vars
    }
    $qemuArgs = @(
        '-machine', 'q35', '-m', "$MemoireMo", '-smp', '2',
        '-drive', "if=pflash,format=raw,unit=0,readonly=on,file=$($ovmf.Code)",
        '-drive', "if=pflash,format=raw,unit=1,file=$vars",
        '-drive', "if=none,id=disque,format=raw,file=$copie",
        '-device', 'nvme,drive=disque,serial=BOUCHAUD_BUREAU',
        '-nic', 'user,model=e1000', '-vga', 'std'
    ) + @(Get-BouchaudDisplayArguments -QemuExe $qemu -Display $Affichage)
    $serie = New-BouchaudQemuSerialPath -RepoRoot $racine -Profile 'bureau' -DryRun:$DryRun
    $qemuArgs += @('-serial', "file:$serie")
    $code = Invoke-BouchaudQemu -QemuExe $qemu -Arguments $qemuArgs -SerialPath $serie -DryRun:$DryRun
    if (-not $DryRun) { Remove-Item -LiteralPath $jetable -Recurse -Force -ErrorAction SilentlyContinue }
    Write-Host "BOUCHAUD_TEST_WINDOWS_FIN mode=bureau code=$code serie=$serie image_inchangee=$((Assert-Empreinte $disque) -eq $empreinte)"
} finally {
    Stop-Transcript | Out-Null
}
