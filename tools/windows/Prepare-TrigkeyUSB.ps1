<#
.SYNOPSIS
  BOUCHAUD_WINDOWS_V1 -- prepare la cle USB du Trigkey, ou installe sur un SSD dedie (via WSL).

.DESCRIPTION
  -Mode Liste : les disques, avec leur identite 'Modele|Serie|Octets'. Rien n'est ecrit.

  -Mode Validation : construit (Linux/WSL) la cle de VALIDATION : le meme
    systeme que l'image de livraison (tools/release/assemble_systeme.sh), demarre
    en live depuis la cle, compile avec `cle-usb-test` : AUCUNE ecriture,
    vidange, formatage ni effacement vers un disque ATA ou NVMe interne
    (docs/current/TRIGKEY_CLE_USB_TEST.md). Puis l'ecrit sur la cle -Disque N.

  -Mode InstallationSSD : rattache le SSD dedie -Disque N a WSL 2
    (`wsl --mount --bare`, droits administrateur) et lance
    tools/install/installe_ssd.py DANS WSL (identite exacte, dependances LVM/
    dm-crypt/RAID/btrfs/swap, simulation, phrase tapee, relecture, STATE
    etendue et initialisee). Ce n'est PAS un installateur natif Bouchaud OS.

  Protections de l'ecriture (Validation) :
  - simulation par DEFAUT ; -Execute pour ecrire ;
  - -Disque N ET -Identite 'Modele|Serie|Octets' EXACTE (copier depuis -Mode Liste) ;
  - refus : disque non USB, disque de demarrage ou systeme, identite differente,
    numero de serie vide, cle plus petite que l'image ;
  - phrase tapee : ECRIRE <SERIE> ;
  - ecriture disque hors ligne, puis RELECTURE et comparaison SHA-256.

  NON QUALIFIE SUR WINDOWS : aucune ecriture physique n'a ete faite par la
  session qui a ecrit ce script. La CI execute sous pwsh les chemins de REFUS.

.EXAMPLE
  .\tools\windows\Prepare-TrigkeyUSB.ps1 -Mode Liste
  .\tools\windows\Prepare-TrigkeyUSB.ps1 -Mode Validation -Disque 3 -Identite 'SanDisk Ultra|4C530001|61530439680'
  .\tools\windows\Prepare-TrigkeyUSB.ps1 -Mode Validation -Disque 3 -Identite '...' -Execute
#>
[CmdletBinding()]
param(
    [ValidateSet('Liste', 'Validation', 'InstallationSSD')][string]$Mode = 'Liste',
    [int]$Disque = -1,
    [string]$Identite = '',
    [string]$Image = '',
    [switch]$Execute
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'BouchaudWindows.psm1') -Force

function Get-IdentiteDisque($d) {
    return ('{0}|{1}|{2}' -f ([string]$d.FriendlyName).Trim(), ([string]$d.SerialNumber).Trim(), [int64]$d.Size)
}

# Les refus, AVANT toute ecriture. Rend l'objet disque si tout est conforme.
function Assert-CleUsb([int]$Numero, [string]$IdentiteAttendue, [int64]$TailleImage) {
    if ($Numero -lt 0) { Stop-Bouchaud '-Disque N obligatoire (voir -Mode Liste) : aucun choix automatique' }
    $d = Get-Disk -Number $Numero -ErrorAction SilentlyContinue
    if (-not $d) { Stop-Bouchaud "disque $Numero introuvable" }
    $lue = Get-IdentiteDisque $d
    if ($IdentiteAttendue -ne $lue) { Stop-Bouchaud "identite differente (lue : '$lue', donnee : '$IdentiteAttendue')" }
    if (-not ([string]$d.SerialNumber).Trim()) { Stop-Bouchaud 'numero de serie illisible : identite non etablie' }
    if ([string]$d.BusType -ne 'USB') { Stop-Bouchaud "disque $Numero non USB ($($d.BusType)) : une cle de validation s'ecrit sur une cle USB" }
    if ($d.IsBoot -or $d.IsSystem) { Stop-Bouchaud "disque $Numero = disque de demarrage ou systeme de Windows" }
    if ([int64]$d.Size -lt $TailleImage) { Stop-Bouchaud "cle trop petite ($($d.Size) < $TailleImage octets)" }
    return $d
}

function Write-ImageBrute([int]$Numero, [string]$Fichier) {
    Set-Disk -Number $Numero -IsOffline $true
    try {
        $src = [System.IO.File]::OpenRead($Fichier)
        $dst = New-Object System.IO.FileStream("\\.\PhysicalDrive$Numero", [System.IO.FileMode]::Open, [System.IO.FileAccess]::ReadWrite)
        try {
            $tampon = New-Object byte[] (4MB)
            while (($n = $src.Read($tampon, 0, $tampon.Length)) -gt 0) {
                if ($n % 512) { $n += 512 - ($n % 512) }
                $dst.Write($tampon, 0, $n)
            }
            $dst.Flush($true)
        } finally { $src.Dispose(); $dst.Dispose() }
        # Relecture de la longueur de l'image, comparee a son empreinte.
        $taille = (Get-Item -LiteralPath $Fichier).Length
        $lu = New-Object System.IO.FileStream("\\.\PhysicalDrive$Numero", [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read)
        $sha = [System.Security.Cryptography.SHA256]::Create()
        try {
            $reste = [int64]$taille
            $tampon = New-Object byte[] (4MB)
            while ($reste -gt 0) {
                $n = $lu.Read($tampon, 0, [int][Math]::Min($tampon.Length, [Math]::Ceiling($reste / 512) * 512))
                if ($n -le 0) { Stop-Bouchaud 'relecture courte' }
                $utile = [int][Math]::Min($n, $reste)
                [void]$sha.TransformBlock($tampon, 0, $utile, $null, 0)
                $reste -= $utile
            }
            [void]$sha.TransformFinalBlock((New-Object byte[] 0), 0, 0)
            return ([BitConverter]::ToString($sha.Hash) -replace '-', '').ToLowerInvariant()
        } finally { $lu.Dispose() }
    } finally {
        Set-Disk -Number $Numero -IsOffline $false
    }
}

$journal = Start-JournalBouchaud "usb-$($Mode.ToLowerInvariant())"
try {
    $racine = Get-RacineDepot
    if ($Mode -eq 'Liste') {
        Get-Disk | Sort-Object Number | ForEach-Object {
            '{0,3}  {1,-6} boot={2} systeme={3}  identite=''{4}''' -f $_.Number, $_.BusType, [int]$_.IsBoot, [int]$_.IsSystem, (Get-IdentiteDisque $_)
        }
        Write-Host 'Aucun disque n''est choisi automatiquement : copier l''identite de la cle (ou du SSD) dans -Identite.'
        return
    }

    if ($Mode -eq 'InstallationSSD') {
        if ($Disque -lt 0) { Stop-Bouchaud '-Disque N obligatoire : aucun choix automatique' }
        if (-not $Image) { $Image = Join-Path $racine 'target/livraison/disque-ab.img' }
        [void](Assert-Empreinte $Image)
        if (-not (Test-SousWindows)) { Stop-Bouchaud 'InstallationSSD : sous Linux, lancer tools/install/installe_ssd.py directement' }
        Write-Etape "rattachement du disque $Disque a WSL 2 (wsl --mount --bare, administrateur requis)"
        & wsl.exe --mount "\\.\PHYSICALDRIVE$Disque" --bare
        if ($LASTEXITCODE -ne 0) { Stop-Bouchaud 'wsl --mount a echoue (droits administrateur ? WSL 2 ?)' }
        try {
            [void](Invoke-Linux 'python3 tools/install/installe_ssd.py liste')
            if (-not $Identite) { Stop-Bouchaud 'relancer avec -Identite ''Modele|Serie|Octets'' telle que listee ci-dessus (vue de WSL)' }
            $cible = Read-Host 'Peripherique WSL du SSD dedie (ex. /dev/sdc), tel que liste ci-dessus'
            $ex = ''
            if ($Execute) { $ex = ' --execute' }
            $code = Invoke-Linux ("python3 tools/install/installe_ssd.py installe --image '{0}' --cible '{1}' --identite '{2}'{3}" -f (ConvertTo-CheminLinux $Image), $cible, $Identite, $ex)
            if ($code -ne 0) { Stop-Bouchaud "installateur : code $code" }
        } finally {
            & wsl.exe --unmount "\\.\PHYSICALDRIVE$Disque"
        }
        Write-Host "BOUCHAUD_INSTALL_SSD_WINDOWS_FIN disque=$Disque execute=$([int][bool]$Execute) journal=$journal"
        return
    }

    # --- Validation : construction de la cle, puis ecriture protegee ----------------
    if (-not $Image) {
        $sha = Get-ShaGit
        $sortie = Join-Path $racine 'target/cle-validation'
        $natif = Join-Path $racine 'native-browser-m9'
        $envLadybird = ''
        if (Test-Path -LiteralPath (Join-Path $natif 'BouchaudBrowserHost')) { $envLadybird = "'$(ConvertTo-CheminLinux $natif)'" }
        $s = ConvertTo-CheminLinux $sortie
        Write-Etape "construction de la cle de validation (commit $sha, disques internes en LECTURE SEULE)"
        # La MEME commande que la CI (trigkey-ab-preflight), qui demarre la cle
        # construite (tools/release/test_cle_validation_qemu.py).
        $code = Invoke-Linux ("bash tools/release/build_cle_validation.sh '$s' $envLadybird")
        if ($code -ne 0) { Stop-Bouchaud "construction de la cle en echec (code $code)" }
        $Image = Join-Path $sortie 'usb/bouchaud-trigkey.img'
    }
    $empreinte = Assert-Empreinte $Image
    $taille = (Get-Item -LiteralPath $Image).Length
    $d = Assert-CleUsb $Disque $Identite $taille
    Write-Host ("PLAN disque={0} identite='{1}' image={2} sha256={3} octets={4} : TOUT LE CONTENU DE LA CLE SERA DETRUIT" -f $Disque, (Get-IdentiteDisque $d), $Image, $empreinte, $taille)
    if (-not $Execute) {
        Write-Host 'USB_DRY_RUN rien n''a ete ecrit (ajouter -Execute)'
        return
    }
    $phrase = "ECRIRE $(([string]$d.SerialNumber).Trim())"
    $saisie = Read-Host "Taper exactement : $phrase"
    if ($saisie -ne $phrase) { Stop-Bouchaud 'confirmation absente ou fausse' }
    [void](Assert-CleUsb $Disque $Identite $taille)
    $relu = Write-ImageBrute $Disque $Image
    if ($relu -ne $empreinte) { Stop-Bouchaud "relecture differente ($relu)" }
    Write-Host "BOUCHAUD_USB_ECRITE disque=$Disque sha256=$empreinte relu=ok journal=$journal PHYSICAL=A_QUALIFIER"
} finally {
    Stop-Transcript | Out-Null
}
