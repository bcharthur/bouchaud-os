param(
    [switch]$Fullscreen,

    # Lance le vrai Ladybird/WebContent natif au lieu du navigateur Qt/Python
    # du userland classique.
    #
    # Usage :
    #   .\run.ps1 -Ladybird          # M9 interactif/persistant
    #   .\run.ps1 -LadybirdM8        # regression M8 finie
    #   .\run.ps1 -LadybirdM9Test    # M9 HTTP deterministe fini
    [switch]$Ladybird,

    [switch]$LadybirdM8,

    [switch]$LadybirdM9Test,

    # Page de depart du mode interactif. Le test M9 remplace toujours cette
    # valeur par sa fixture deterministe, sauf surcharge explicite.
    [string]$LadybirdUrl = "https://example.com/",

    # Retire la barre d'outils M11 et revient au comportement de M9 : une seule
    # capture, aucune entree. Utile pour isoler une regression entre le moteur
    # et le chrome : est-ce la page, ou est-ce la barre ?
    [switch]$LadybirdSansChrome,

    # Profil materiel du navigateur natif.
    # 8192 Mio est volontairement le profil local par defaut : Bouchaud peut
    # utiliser la RAM supplementaire, contrairement aux vCPU additionnels qui
    # attendent encore le vrai port SMP/APIC.
    [ValidateRange(2048, 16384)]
    [int]$LadybirdRamMiB = 8192,

    # Expose la topologie QEMU pour preparer le futur SMP. Le noyau actuel ne
    # schedule encore que sur le BSP : >1 vCPU n'accelere donc PAS encore
    # WebContent. Garder 1 tant que SMP/APIC n'est pas merge.
    [ValidateRange(1, 16)]
    [int]$LadybirdCpuCount = 1,

    # Force le retelechargement de l'artefact Ladybird depuis GitHub Actions.
    [switch]$RefreshLadybird,

    # Run GitHub Actions contenant l'artefact Ladybird.
    # 0 = dernier run reussi de ladybird-native-browser sur la branche courante.
    [long]$LadybirdRunId = 0,

    # Sortie audio hote.
    # Windows : "dsound"
    # Muet    : "none"
    [string]$Audio = "dsound",

    # Acceleration QEMU.
    #
    # auto :
    #   WHPX puis fallback TCG.
    #
    # none :
    #   pas d'acceleration explicite.
    [string]$Accel = "auto",

    # Met a jour la branche courante avant construction.
    [switch]$Sync,

    # Mode userland classique uniquement.
    [switch]$NoUserlandDownload,

    # Mode userland classique uniquement.
    [switch]$RefreshUserland,

    # Mode userland classique uniquement.
    [switch]$AllowOlderUserland
)

$ErrorActionPreference = "Stop"

$RepoRoot = $PSScriptRoot

Set-Location $RepoRoot


# =============================================================================
# Helpers
# =============================================================================

function Write-Section {
    param(
        [string]$Text
    )

    Write-Host ""
    Write-Host "=== $Text ===" -ForegroundColor Cyan
}


function Fail {
    param(
        [string]$Message
    )

    Write-Host ""
    Write-Host "ERREUR: $Message" -ForegroundColor Red

    exit 1
}


function ConvertTo-ShellSingleQuoted {
    param(
        [string]$Value
    )

    # L'autorun est interprete par /bin/sh dans l'OS. Une apostrophe ne peut pas
    # apparaitre telle quelle dans une chaine entre apostrophes : on ferme la
    # chaine, on emet une apostrophe echappee, puis on la rouvre, soit la
    # sequence POSIX \'\''. Ne jamais injecter directement une valeur venant du
    # terminal.
    #
    # L'apostrophe est nommee au lieu d'etre ecrite : PowerShell n'echappe pas
    # le guillemet avec une barre oblique inverse, et la version qui essayait
    # rendait TOUT le script inanalysable - `.\run.ps1` refusait de demarrer,
    # quel que soit le mode demande.
    $apostrophe = [string][char]39
    $echappee = $apostrophe + '\' + $apostrophe + $apostrophe

    return $apostrophe + $Value.Replace($apostrophe, $echappee) + $apostrophe
}


function Ensure-Cargo {

    $cargoBin = Join-Path `
        $env:USERPROFILE `
        ".cargo\bin"

    if (Test-Path $cargoBin) {

        $pathEntries = $env:Path -split ";"

        if ($pathEntries -notcontains $cargoBin) {

            $env:Path = "$cargoBin;$env:Path"
        }
    }

    $cargoCommand = Get-Command `
        cargo `
        -ErrorAction SilentlyContinue

    if (-not $cargoCommand) {

        Fail "cargo introuvable. Chemin attendu : $cargoBin"
    }

    $cargoVersion = & cargo --version

    Write-Host `
        "cargo : $cargoVersion" `
        -ForegroundColor DarkGray
}


function Ensure-Python {

    $pythonCommand = Get-Command `
        python `
        -ErrorAction SilentlyContinue

    if (-not $pythonCommand) {

        Fail "Python Windows est introuvable."
    }

    $pythonVersion = & python --version

    if ($LASTEXITCODE -ne 0) {

        Fail "Python est present mais ne peut pas etre execute."
    }

    Write-Host `
        $pythonVersion `
        -ForegroundColor DarkGray
}


function Ensure-GitHubCli {

    $ghCommand = Get-Command `
        gh `
        -ErrorAction SilentlyContinue

    if (-not $ghCommand) {

        Fail "GitHub CLI 'gh' est introuvable."
    }
}


# =============================================================================
# Selection du mode Ladybird
# =============================================================================

$LadybirdModeCount = @(
    [bool]$Ladybird,
    [bool]$LadybirdM8,
    [bool]$LadybirdM9Test
).Where({ $_ }).Count

if ($LadybirdModeCount -gt 1) {
    Fail "utiliser un seul mode parmi -Ladybird, -LadybirdM8, -LadybirdM9Test"
}

$LadybirdMode = $Ladybird -or $LadybirdM8 -or $LadybirdM9Test

if ($LadybirdM9Test -and -not $PSBoundParameters.ContainsKey("LadybirdUrl")) {
    $LadybirdUrl = "http://10.0.2.2:18080/m9.html"
}

# Le chrome M11 n'a de sens que dans le mode interactif. Le test M9 mesure le
# moteur : lui ajouter une barre d'outils changerait ce qu'il mesure.
$LadybirdChrome = $Ladybird -and -not $LadybirdSansChrome

if ($Ladybird -or $LadybirdM9Test) {
    $parsedLadybirdUrl = $null
    if (-not [System.Uri]::TryCreate(
        $LadybirdUrl,
        [System.UriKind]::Absolute,
        [ref]$parsedLadybirdUrl
    )) {
        Fail "LadybirdUrl doit etre une URL absolue http:// ou https://"
    }

    if ($parsedLadybirdUrl.Scheme -notin @("http", "https")) {
        Fail (
            "Ladybird accepte http:// et https:// uniquement, pas '{0}'" -f `
                $parsedLadybirdUrl.Scheme
        )
    }
}


# =============================================================================
# Etat de la source
# =============================================================================

& "$RepoRoot\tools\etat-source.ps1" `
    -RepoRoot $RepoRoot `
    -Sync:$Sync

if ($LASTEXITCODE -ne 0) {

    Fail "etat-source.ps1 a echoue."
}


# =============================================================================
# Construction du noyau
# =============================================================================

Write-Section "Construction du noyau"

Ensure-Cargo

cargo bootimage

if ($LASTEXITCODE -ne 0) {

    Fail "cargo bootimage a echoue (code $LASTEXITCODE). QEMU ne sera pas lance."
}


$BootImage = Join-Path `
    $RepoRoot `
    "target\x86_64-bouchaud_os\debug\bootimage-bouchaud-os.bin"


if (-not (Test-Path $BootImage)) {

    Fail "bootimage introuvable : $BootImage"
}


$BootInfo = Get-Item $BootImage

Write-Host (
    "bootimage : {0} ({1:N1} Mio)" -f `
        $BootInfo.FullName, `
        ($BootInfo.Length / 1MB)
) -ForegroundColor Green


# =============================================================================
# Arguments QEMU de base
# =============================================================================

$qemuArgs = @(
    "-drive",
    "format=raw,file=$BootImage"
)


# =============================================================================
# Mode Ladybird
# =============================================================================

if ($LadybirdMode) {

    Write-Section "Ladybird natif"

    Ensure-Python

    $IsLadybirdM8 = [bool]$LadybirdM8
    $IsLadybirdM9Test = [bool]$LadybirdM9Test
    $IsLadybirdM9 = -not $IsLadybirdM8

    $NativeBrowserName = if ($IsLadybirdM8) { "native-browser-m8" } else { "native-browser-m9" }
    $ScenarioName = if ($IsLadybirdM8) { "scenario-m8" } else { "scenario-m9" }


    $NativeBrowserDir = Join-Path `
        $RepoRoot `
        $NativeBrowserName


    $LadybirdImage = Join-Path `
        $RepoRoot `
        "ladybird-browser.img"


    $ScenarioDir = Join-Path `
        $RepoRoot `
        $ScenarioName


    # -------------------------------------------------------------------------
    # M8 n'a besoin QUE de WebContent + bootstrap.
    #
    # RequestServer :
    #   inutile avant M9 reseau.
    #
    # ImageDecoder :
    #   inutile pour la page HTML locale M8.
    #
    # WebWorker :
    #   inutile pour M8.
    #
    # Cela evite de gonfler le disque a ~654 Mio.
    # -------------------------------------------------------------------------

    if ($IsLadybirdM8) {
        $RequiredLadybirdFiles = @(
            "WebContent",
            "webcontent-bootstrap"
        )
    }
    else {
        $RequiredLadybirdFiles = @(
            "WebContent",
            "RequestServer",
            "webcontent-bootstrap",
            "M9_CAPABLE"
        )
    }


    # =========================================================================
    # Validation / recuperation artefact Ladybird
    # =========================================================================

    $artifactMissing = $false


    if (-not (Test-Path $NativeBrowserDir)) {

        $artifactMissing = $true
    }
    else {

        foreach ($file in $RequiredLadybirdFiles) {

            $candidate = Join-Path `
                $NativeBrowserDir `
                $file

            if (-not (Test-Path $candidate)) {

                $artifactMissing = $true

                break
            }
        }


        $resourcesCandidate = Join-Path `
            $NativeBrowserDir `
            "resources"


        if (-not (Test-Path $resourcesCandidate)) {

            $artifactMissing = $true
        }
    }


    if ($RefreshLadybird -or $artifactMissing) {

        Ensure-GitHubCli


        $EffectiveLadybirdRunId = $LadybirdRunId

        if ($EffectiveLadybirdRunId -eq 0) {
            $CurrentBranch = (& git branch --show-current).Trim()
            if (-not $CurrentBranch) {
                Fail "impossible de determiner la branche pour trouver l'artefact Ladybird"
            }

            $latest = (& gh run list `
                --workflow "ladybird-native-browser.yml" `
                --branch $CurrentBranch `
                --status success `
                --limit 1 `
                --json databaseId `
                --jq '.[0].databaseId').Trim()

            if (-not $latest) {
                Fail "aucun build Ladybird reussi pour '$CurrentBranch'. Pousse la branche et attends le workflow ladybird-native-browser."
            }

            $EffectiveLadybirdRunId = [long]$latest
        }

        Write-Host (
            "Ladybird : telechargement artefact du run {0}" -f `
                $EffectiveLadybirdRunId
        ) -ForegroundColor Yellow


        if (Test-Path $NativeBrowserDir) {

            Remove-Item `
                -Recurse `
                -Force `
                $NativeBrowserDir
        }


        New-Item `
            -ItemType Directory `
            -Path $NativeBrowserDir `
            -Force | Out-Null


        gh run download $EffectiveLadybirdRunId `
            -n "bouchaud-ladybird-native-browser" `
            -D $NativeBrowserDir


        if ($LASTEXITCODE -ne 0) {

            Fail "impossible de telecharger l'artefact Ladybird du run $EffectiveLadybirdRunId"
        }
    }
    else {

        Write-Host (
            "Ladybird : artefact local deja present dans {0}" -f `
                $NativeBrowserDir
        ) -ForegroundColor Green
    }


    # =========================================================================
    # Validation minimale M8
    # =========================================================================

    foreach ($file in $RequiredLadybirdFiles) {

        $path = Join-Path `
            $NativeBrowserDir `
            $file


        if (-not (Test-Path $path)) {

            Fail "artefact Ladybird incomplet : $file absent"
        }
    }


    $ResourcesDir = Join-Path `
        $NativeBrowserDir `
        "resources"


    if (-not (Test-Path $ResourcesDir)) {

        Fail "artefact Ladybird incomplet : resources absent"
    }


    $WebContentPath = Join-Path `
        $NativeBrowserDir `
        "WebContent"


    $WebContentInfo = Get-Item $WebContentPath


    # Garde synchronisee avec le plafond actuel du noyau.
    $MaxBootFile = 512MB


    if ($WebContentInfo.Length -gt $MaxBootFile) {

        Fail (
            "WebContent fait {0:N1} Mio, limite Bouchaud actuelle : 512 Mio" -f `
                ($WebContentInfo.Length / 1MB)
        )
    }


    Write-Host (
        "WebContent : {0:N1} Mio" -f `
            ($WebContentInfo.Length / 1MB)
    ) -ForegroundColor DarkGray


    # =========================================================================
    # Fabrication du scenario M8
    # =========================================================================

    Write-Section "Fabrication du disque Ladybird"


    if (Test-Path $ScenarioDir) {

        Remove-Item `
            -Recurse `
            -Force `
            $ScenarioDir
    }


    $LadybirdLibexec = Join-Path `
        $ScenarioDir `
        "usr\libexec\ladybird"


    $LadybirdShare = Join-Path `
        $ScenarioDir `
        "usr\share\ladybird"


    New-Item `
        -ItemType Directory `
        -Path $LadybirdLibexec `
        -Force | Out-Null


    New-Item `
        -ItemType Directory `
        -Path $LadybirdShare `
        -Force | Out-Null


    # =========================================================================
    # WebContent
    # =========================================================================

    Copy-Item `
        (Join-Path $NativeBrowserDir "WebContent") `
        (Join-Path $LadybirdLibexec "WebContent")

    if ($IsLadybirdM9) {
        Copy-Item `
            (Join-Path $NativeBrowserDir "RequestServer") `
            (Join-Path $LadybirdLibexec "RequestServer")
    }


    # =========================================================================
    # Bootstrap
    # =========================================================================

    Copy-Item `
        (Join-Path $NativeBrowserDir "webcontent-bootstrap") `
        (Join-Path $LadybirdLibexec "webcontent-bootstrap")


    # =========================================================================
    # /bo-navigateur
    #
    # IMPORTANT :
    #
    # En mode -Ladybird, /bo-navigateur n'est PAS le vieux navigateur
    # Qt/Python.
    #
    # Il s'agit du bootstrap qui engendre le vrai WebContent Ladybird.
    # =========================================================================

    Copy-Item `
        (Join-Path $NativeBrowserDir "webcontent-bootstrap") `
        (Join-Path $ScenarioDir "bo-navigateur")


    # =========================================================================
    # Ressources Ladybird
    #
    # Fontes SerenitySans, resource://, etc.
    # =========================================================================

    Copy-Item `
        (Join-Path $ResourcesDir "*") `
        $LadybirdShare `
        -Recurse `
        -Force


    # =========================================================================
    # Certificats publics pour HTTPS Internet
    # =========================================================================

    if ($IsLadybirdM9) {
        $PublicCABundle = Join-Path `
            $RepoRoot `
            "tools\ladybird\certs\cacert.pem"

        # Sans magasin d'autorites, RequestServer laisse curl retomber sur un
        # chemin decide a la compilation, qui n'existe pas ici : chaque poignee
        # de main TLS echoue, et le symptome ressemble a une panne reseau. On le
        # fabrique donc au lieu d'attendre que l'utilisateur devine.
        if (-not (Test-Path -LiteralPath $PublicCABundle -PathType Leaf)) {
            $Fabrique = Join-Path `
                $RepoRoot `
                "tools\ladybird\certs\fabrique-bundle.ps1"

            if (Test-Path -LiteralPath $Fabrique -PathType Leaf) {
                Write-Host `
                    "CA HTTPS   : fabrication du magasin d'autorites..." `
                    -ForegroundColor DarkGray

                & $Fabrique -Sortie $PublicCABundle | Out-Null
            }
        }

        if (Test-Path -LiteralPath $PublicCABundle -PathType Leaf) {
            $ScenarioCertDir = Join-Path `
                $ScenarioDir `
                "etc\ssl\certs"

            New-Item `
                -ItemType Directory `
                -Path $ScenarioCertDir `
                -Force | Out-Null

            Copy-Item `
                -LiteralPath $PublicCABundle `
                -Destination (Join-Path $ScenarioCertDir "ca-certificates.crt") `
                -Force

            Write-Host `
                "CA HTTPS   : Mozilla/curl -> /etc/ssl/certs/ca-certificates.crt" `
                -ForegroundColor DarkGray
        }
        elseif ($LadybirdUrl -match '^https://') {
            Fail (
                ("URL HTTPS demandee mais aucun magasin d'autorites : {0}. " +
                 "Lancer tools\ladybird\certs\fabrique-bundle.ps1, ou " +
                 "demander une URL http://.") -f $PublicCABundle
            )
        }
    }


    # =========================================================================
    # Autorun
    # =========================================================================

    $AutorunPath = Join-Path `
        $ScenarioDir `
        "autorun"


    # LF uniquement.
    #
    # Pas de CRLF Windows dans le fichier execute dans Bouchaud OS.
    if ($IsLadybirdM8) {
        $autorun = @(
            'uname',
            'df',
            'echo "=== Ladybird M8 : HTML local dans fenetre Bouchaud ==="',
            'export BO_AUTOSTART_BROWSER=1',
            'export BOUCHAUD_M8=1',
            'desktop',
            ''
        ) -join "`n"
    }
    else {
        $m9TestLine = if ($IsLadybirdM9Test) { 'export BOUCHAUD_M9_TEST=1' } else { 'echo "Navigateur : fermer la fenetre pour quitter"' }
        $chromeLine = if ($LadybirdChrome) { 'export BOUCHAUD_M11=1' } else { 'echo "M11 desactive : capture unique, sans entrees"' }
        $banniere = if ($LadybirdChrome) {
            'echo "=== Bouchaud Navigateur (Ladybird M11) ==="'
        }
        else {
            'echo "=== Ladybird M9 : HTTP distant via RequestServer ==="'
        }
        $autorun = @(
            'uname',
            'df',
            $banniere,
            'export BO_AUTOSTART_BROWSER=1',
            'export BOUCHAUD_M9=1',
            "export BOUCHAUD_M9_URL=$(ConvertTo-ShellSingleQuoted $LadybirdUrl)",
            $chromeLine,
            $m9TestLine,
            'desktop',
            ''
        ) -join "`n"
    }


    [System.IO.File]::WriteAllText(
        $AutorunPath,
        $autorun,
        [System.Text.UTF8Encoding]::new($false)
    )


    # =========================================================================
    # Fabrication USTAR native Windows
    #
    # AUCUN WSL.
    # AUCUN Linux.
    #
    # Python sert uniquement d'outil de packaging hote.
    #
    # Execution finale :
    #
    # Windows
    #   ->
    # QEMU
    #   ->
    # Bouchaud OS bare-metal
    #   ->
    # WebContent Ladybird
    # =========================================================================

    $TempBuilder = Join-Path `
        $env:TEMP `
        "bouchaud-make-ladybird-image.py"


    $PythonBuilder = @'
import sys
import tarfile
from pathlib import Path


if len(sys.argv) != 3:
    raise SystemExit(
        "usage: bouchaud-make-ladybird-image.py <scenario> <image>"
    )


root = Path(sys.argv[1]).resolve()
output = Path(sys.argv[2]).resolve()


if not root.is_dir():
    raise SystemExit(
        f"scenario absent: {root}"
    )


# M8 minimal :
#
# Pas de RequestServer.
# Pas d'ImageDecoder.
# Pas de WebWorker.
#
# Ils seront introduits quand leur jalon les utilisera vraiment.
executables = {
    "bo-navigateur",
    "usr/libexec/ladybird/WebContent",
    "usr/libexec/ladybird/RequestServer",
    "usr/libexec/ladybird/webcontent-bootstrap",
}


if output.exists():
    output.unlink()


print(
    "[ladybird-image] scenario :",
    root
)

print(
    "[ladybird-image] sortie   :",
    output
)

print(
    "[ladybird-image] format   : USTAR"
)


with tarfile.open(
    output,
    mode="w",
    format=tarfile.USTAR_FORMAT,
) as archive:

    paths = sorted(
        root.rglob("*"),
        key=lambda path: path.as_posix(),
    )


    for path in paths:

        relative = path.relative_to(root).as_posix()

        archive_name = f"./{relative}"


        info = archive.gettarinfo(
            str(path),
            arcname=archive_name,
        )


        # Metadata deterministe.
        info.uid = 0
        info.gid = 0
        info.uname = "root"
        info.gname = "root"


        if path.is_dir():

            info.mode = 0o755

            archive.addfile(info)

            continue


        if relative in executables:

            info.mode = 0o755

        else:

            info.mode = 0o644


        with path.open("rb") as stream:

            archive.addfile(
                info,
                stream,
            )


# ============================================================================
# Alignement disque
# ============================================================================

size = output.stat().st_size

padding = (-size) % 512


with output.open("ab") as stream:

    if padding:

        stream.write(
            b"\0" * padding
        )


    # ========================================================================
    # Zone persistante Bouchaud
    #
    # 16384 secteurs * 512 = 8 Mio.
    # ========================================================================

    remaining = 8 * 1024 * 1024

    zero_block = b"\0" * (
        1024 * 1024
    )


    while remaining:

        chunk = min(
            remaining,
            len(zero_block),
        )

        stream.write(
            zero_block[:chunk]
        )

        remaining -= chunk


final_size = output.stat().st_size


if final_size % 512:

    raise SystemExit(
        "image finale non alignee sur 512 octets"
    )


print(
    "[ladybird-image] taille   : "
    f"{final_size:,} octets "
    f"({final_size / 1024 / 1024:.1f} Mio)"
)


print(
    "[ladybird-image] secteurs : "
    f"{final_size // 512:,}"
)


print(
    "[ladybird-image] OK"
)
'@


    [System.IO.File]::WriteAllText(
        $TempBuilder,
        $PythonBuilder,
        [System.Text.UTF8Encoding]::new($false)
    )


    try {

        python `
            $TempBuilder `
            $ScenarioDir `
            $LadybirdImage


        if ($LASTEXITCODE -ne 0) {

            Fail "fabrication de ladybird-browser.img echouee."
        }
    }
    finally {

        Remove-Item `
            $TempBuilder `
            -Force `
            -ErrorAction SilentlyContinue
    }


    # =========================================================================
    # Validation image
    # =========================================================================

    if (-not (Test-Path $LadybirdImage)) {

        Fail "ladybird-browser.img n'a pas ete genere."
    }


    $LadybirdImageInfo = Get-Item $LadybirdImage


    # Garde haute du noyau.
    $MaxLadybirdDisk = 768MB


    if ($LadybirdImageInfo.Length -gt $MaxLadybirdDisk) {

        Fail (
            "ladybird-browser.img fait {0:N1} Mio ; limite actuelle 768 Mio." -f `
                ($LadybirdImageInfo.Length / 1MB)
        )
    }


    Write-Host (
        "disque Ladybird : {0} ({1:N1} Mio)" -f `
            $LadybirdImageInfo.FullName, `
            ($LadybirdImageInfo.Length / 1MB)
    ) -ForegroundColor Green


    # =========================================================================
    # Second disque QEMU
    #
    # On n'attache PAS tools/userland/userland.img.
    #
    # Celui-ci contient encore le vieux navigateur Qt/Python.
    # =========================================================================

    $qemuArgs += @(
        "-drive",
        "format=raw,file=$LadybirdImage"
    )
}
else {

    # =========================================================================
    # Mode Bouchaud OS classique
    # =========================================================================

    Write-Section "Userland"


    & "$RepoRoot\tools\userland.ps1" `
        -RepoRoot $RepoRoot `
        -NoDownload:$NoUserlandDownload `
        -Refresh:$RefreshUserland `
        -AllowOlder:$AllowOlderUserland


    $userland = Join-Path `
        $RepoRoot `
        "tools\userland\userland.img"


    if (Test-Path $userland) {

        Write-Host (
            "disque userland : {0}" -f `
                $userland
        ) -ForegroundColor Cyan


        $qemuArgs += @(
            "-drive",
            "format=raw,file=$userland"
        )
    }
}


# =============================================================================
# RAM
# =============================================================================

if ($LadybirdMode) {

    $qemuArgs += @(
        "-m",
        "$LadybirdRamMiB"
    )

    $qemuArgs += @(
        "-smp",
        "$LadybirdCpuCount"
    )

    if ($LadybirdCpuCount -gt 1) {
        Write-Host `
            "ATTENTION: $LadybirdCpuCount vCPU exposes, mais Bouchaud est encore BSP/PIC ; SMP guest n'est pas encore actif." `
            -ForegroundColor Yellow
    }
}
else {

    $qemuArgs += @(
        "-m",
        "2048"
    )
}


# =============================================================================
# Serie
# =============================================================================

$qemuArgs += @(
    "-serial",
    "stdio"
)


# =============================================================================
# Reseau
# =============================================================================

$qemuArgs += @(
    "-netdev",
    "user,id=net0",

    "-device",
    "e1000,netdev=net0"
)


# =============================================================================
# Audio
# =============================================================================

if ($Audio -eq "none") {

    $qemuArgs += @(
        "-audiodev",
        "none,id=snd0",

        "-device",
        "AC97,audiodev=snd0"
    )
}
else {

    $qemuArgs += @(
        "-audiodev",
        "$Audio,id=snd0",

        "-device",
        "AC97,audiodev=snd0"
    )
}


# =============================================================================
# Acceleration QEMU
# =============================================================================

if ($Accel -ne "none") {

    if ($Accel -eq "auto") {

        # Windows Hypervisor Platform.
        #
        # Si indisponible, QEMU retombe sur TCG.
        $qemuArgs += @(
            "-accel",
            "whpx,kernel-irqchip=off",

            "-accel",
            "tcg"
        )
    }
    else {

        $qemuArgs += @(
            "-accel",
            $Accel
        )
    }
}


# =============================================================================
# Plein ecran
# =============================================================================

if ($Fullscreen) {

    $qemuArgs += "-full-screen"
}


# =============================================================================
# Demarrage QEMU
# =============================================================================

Write-Section "Demarrage QEMU"


$QemuExe = `
    "C:\Program Files\qemu\qemu-system-x86_64.exe"


if (-not (Test-Path $QemuExe)) {

    Fail "QEMU introuvable : $QemuExe"
}


if ($LadybirdMode) {

    if ($IsLadybirdM8) {
        $ModeLabel = "Ladybird natif M8"
        $ServiceLabel = "WebContent uniquement (regression M8)"
        $BrowserChain = "/bo-navigateur -> bootstrap -> WebContent"
    }
    elseif ($IsLadybirdM9Test) {
        $ModeLabel = "Ladybird natif M9 TEST HTTP"
        $ServiceLabel = "WebContent + RequestServer"
        $BrowserChain = "/bo-navigateur -> bootstrap -> RequestServer + WebContent"
    }
    elseif ($LadybirdChrome) {
        $ModeLabel = "Bouchaud Navigateur (Ladybird M11)"
        $ServiceLabel = "WebContent + RequestServer"
        $BrowserChain = "/bo-navigateur -> bootstrap -> RequestServer + WebContent + chrome"
    }
    else {
        $ModeLabel = "Ladybird natif M9 interactif (sans chrome)"
        $ServiceLabel = "WebContent + RequestServer"
        $BrowserChain = "/bo-navigateur -> bootstrap -> RequestServer + WebContent"
    }

    Write-Host `
        "mode       : $ModeLabel" `
        -ForegroundColor Green

    Write-Host `
        "RAM        : $LadybirdRamMiB Mio"

    Write-Host `
        "vCPU       : $LadybirdCpuCount (SMP guest: non, BSP actuel)"

    Write-Host `
        "navigateur : $BrowserChain"

    Write-Host `
        "services   : $ServiceLabel"

    if ($IsLadybirdM9) {
        Write-Host `
            "URL        : $LadybirdUrl"
    }

    if ($LadybirdChrome) {
        Write-Host `
            "chrome     : barre d'adresse, historique, liens, defilement"

        Write-Host `
            "clavier    : cliquer la barre pour la saisir, Entree pour aller, Echap pour rendre le foyer" `
            -ForegroundColor DarkGray
    }

    Write-Host `
        "Linux/WSL  : aucun"

    Write-Host ""
}
else {

    Write-Host `
        "mode       : Bouchaud OS classique"

    Write-Host `
        "RAM        : 2048 Mio"

    Write-Host ""
}


Write-Host `
    "QEMU :" `
    -ForegroundColor DarkGray


Write-Host `
    $QemuExe `
    -ForegroundColor DarkGray


foreach ($arg in $qemuArgs) {

    Write-Host `
        "  $arg" `
        -ForegroundColor DarkGray
}


Write-Host ""


$FixtureProcess = $null
$FixtureOut = Join-Path $env:TEMP "bouchaud-m9-fixture.out.log"
$FixtureErr = Join-Path $env:TEMP "bouchaud-m9-fixture.err.log"

# La fixture locale sert le test M9 deterministe, et rien d'autre. Le mode
# interactif sort par le NAT de QEMU : la demarrer alors ouvrirait un port sur
# l'hote sans qu'aucun code ne le consulte.
$UseLocalM9Fixture = $LadybirdMode -and $IsLadybirdM9 -and (
    $IsLadybirdM9Test -or
    $LadybirdUrl.StartsWith("http://10.0.2.2:18080/")
)

if ($UseLocalM9Fixture) {
    Remove-Item $FixtureOut, $FixtureErr -Force -ErrorAction SilentlyContinue

    $FixtureScript = Join-Path $RepoRoot "tools\health\fixture_server.py"
    $FixtureProcess = Start-Process `
        -FilePath "python" `
        -ArgumentList @($FixtureScript, "--port", "18080") `
        -RedirectStandardOutput $FixtureOut `
        -RedirectStandardError $FixtureErr `
        -WindowStyle Hidden `
        -PassThru

    Start-Sleep -Milliseconds 600

    if ($FixtureProcess.HasExited) {
        if (Test-Path $FixtureErr) { Get-Content $FixtureErr }
        Fail "fixture HTTP M9 n'a pas demarre"
    }

    Write-Host "fixture M9 : http://10.0.2.2:18080/m9.html" -ForegroundColor DarkGray
}

try {
    & $QemuExe @qemuArgs
    $QemuExitCode = $LASTEXITCODE
}
finally {
    if ($FixtureProcess -and -not $FixtureProcess.HasExited) {
        Stop-Process -Id $FixtureProcess.Id -Force -ErrorAction SilentlyContinue
        $FixtureProcess.WaitForExit()
    }
}

if ($UseLocalM9Fixture) {
    Write-Host ""
    Write-Host "=== Journal fixture M9 ===" -ForegroundColor DarkGray
    if (Test-Path $FixtureOut) {
        Get-Content $FixtureOut
    }
    if (Test-Path $FixtureErr) {
        Get-Content $FixtureErr
    }

    if ($IsLadybirdM9Test) {
        $fixtureText = if (Test-Path $FixtureOut) { Get-Content $FixtureOut -Raw } else { "" }
        if ($fixtureText -notmatch "M9_FIXTURE_HTTP_OK") {
            Fail "M9 test: aucune requete /m9.html recue par la fixture hote"
        }
    }
}

Write-Host ""

Write-Host (
    "QEMU termine avec le code {0}" -f `
        $QemuExitCode
) -ForegroundColor DarkGray


exit $QemuExitCode
