param(
    [switch]$Fullscreen,

    # Bouchaud OS demarre desormais sur son bureau avec le runtime Ladybird
    # disponible : `.\run.ps1` suffit, et le navigateur s'ouvre par un
    # double-clic sur l'icone "Navigateur". Ce commutateur ne sert plus qu'a
    # ne pas casser les habitudes et les documents qui le mentionnent.
    [switch]$Ladybird,

    # Revient au userland historique (Qt + CPython + QuickJS). Le navigateur du
    # bureau est alors l'ancien moteur, pas Ladybird.
    [switch]$Legacy,

    [switch]$LadybirdM8,

    [switch]$LadybirdM9Test,

    # Page de depart du mode interactif.
    #
    # La raison de choisir une fixture locale a disparu : elle tenait a ce que
    # resoudre un nom bloquait cinq minutes, ce que la correction DNS a leve
    # (`tools/ladybird/prepare-dns-une-question.py`). Un navigateur dont la page
    # d'accueil vit sur la machine de developpement n'est pas un navigateur.
    #
    # `example.com` plutot qu'un site riche : c'est la seule page publique dont
    # la chaine complete - nom, DNS, TCP, TLS, HTTP, analyse, mise en page,
    # peinture, ecran - soit verte en integration continue. La barre d'adresse
    # est la pour le reste, et elle marche.
    [string]$LadybirdUrl = "https://example.com/",

    # Retire la barre d'outils M11 et revient au comportement de M9 : une seule
    # capture, aucune entree. Utile pour isoler une regression entre le moteur
    # et le chrome : est-ce la page, ou est-ce la barre ?
    [switch]$LadybirdSansChrome,

    # Memoire donnee a la machine.
    #
    # 12288 Mio est la plus grande valeur **verifiee** : le noyau demarre, la
    # sonde reseau passe, et le temps de demarrage ne prend que trois secondes
    # de plus qu'a 2048 Mio. 16384 reste accepte mais n'a pas pu etre eprouve
    # ici, faute d'un hote assez grand - voir docs/ladybird/M13_DNS.md.
    [ValidateRange(2048, 16384)]
    [Alias('LadybirdRamMiB')]
    [int]$RamMiB = 12288,

    # Nombre de vCPU exposes.
    #
    # **Un seul, et ce n'est pas une timidite.** Le noyau ne sait pas demarrer
    # un second processeur : il ne lit ni ACPI ni la table MADT, n'a pas de
    # LAPIC, ne peut donc pas emettre la sequence INIT/SIPI, et route ses
    # interruptions par le PIC 8259, qui ne parle qu'au BSP. `kernel::task`
    # tient par ailleurs une file d'ordonnancement unique sans verrou.
    # Demander huit vCPU donnerait huit coeurs a QEMU dont sept resteraient
    # eteints, et ferait croire a une acceleration qui n'existe pas.
    [ValidateRange(1, 16)]
    [Alias('LadybirdCpuCount')]
    [int]$CpuCount = 4,

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

$ModeCount = @(
    [bool]$Legacy,
    [bool]$LadybirdM8,
    [bool]$LadybirdM9Test
).Where({ $_ }).Count

if ($ModeCount -gt 1) {
    Fail "utiliser un seul mode parmi -Legacy, -LadybirdM8, -LadybirdM9Test"
}

# Ladybird est le mode **normal**. `-Legacy` rend la main au userland
# historique ; les deux autres sont les regressions de la CI.
$LadybirdMode = -not $Legacy

if ($LadybirdM9Test -and -not $PSBoundParameters.ContainsKey("LadybirdUrl")) {
    $LadybirdUrl = "http://10.0.2.2:18080/m9.html"
}

# Le chrome M11 n'a de sens que dans le mode interactif. Le test M9 mesure le
# moteur : lui ajouter une barre d'outils changerait ce qu'il mesure.
$LadybirdInteractif = $LadybirdMode -and -not ($LadybirdM8 -or $LadybirdM9Test)
$LadybirdChrome = $LadybirdInteractif -and -not $LadybirdSansChrome

if ($LadybirdInteractif -or $LadybirdM9Test) {
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
    # Services embarques.
    #
    # ImageDecoder n'est pas facultatif, meme pour une page locale : c'est lui
    # qui installe `Web::Platform::ImageCodecPlugin` dans WebContent. Sans le
    # greffon, la premiere balise <img> rencontree fait tomber
    # `VERIFY(s_the)` - c'est ce qui tuait WebContent sur Wikipedia.
    #
    # RequestServer :
    #   DNS, TCP, TLS, HTTP. Inutile pour la page locale.
    #
    # WebWorker :
    #   pas encore lance : il exige un processus hote capable de repondre au
    #   message synchrone `StartWorkerAgent`. Voir
    #   docs/ladybird/AUDIT_INTEGRATION.md.
    # -------------------------------------------------------------------------

    if ($IsLadybirdM8) {
        $RequiredLadybirdFiles = @(
            "WebContent",
            "ImageDecoder",
            "webcontent-bootstrap"
        )
    }
    elseif ($LadybirdInteractif) {
        $RequiredLadybirdFiles = @(
            "BouchaudBrowserHost",
            "WebContent",
            "RequestServer",
            "ImageDecoder",
            "Compositor",
            "WebWorker",
            "WebDriver",
            "webcontent-bootstrap",
            "M9_CAPABLE"
        )
    }
    else {
        $RequiredLadybirdFiles = @(
            "WebContent",
            "RequestServer",
            "ImageDecoder",
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
    # ImageDecoder
    #
    # Toujours copie : les images ne dependent pas du jalon reseau.
    # =========================================================================

    Copy-Item `
        (Join-Path $NativeBrowserDir "ImageDecoder") `
        (Join-Path $LadybirdLibexec "ImageDecoder")

    if ($LadybirdInteractif) {
        foreach ($service in @("Compositor", "WebWorker", "WebDriver", "BouchaudBrowserHost")) {
            Copy-Item `
                (Join-Path $NativeBrowserDir $service) `
                (Join-Path $LadybirdLibexec $service)
        }
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

    $BrowserEntry = if ($LadybirdInteractif) {
        "BouchaudBrowserHost"
    }
    else {
        "webcontent-bootstrap"
    }

    Copy-Item `
        (Join-Path $NativeBrowserDir $BrowserEntry) `
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
    # Configuration fontconfig
    #
    # Elle voyage normalement avec les ressources de l'artefact. On la repose
    # depuis le depot quand l'artefact est anterieur a son introduction : sans
    # elle, le gestionnaire de polices de Skia ne voit aucune police et tout le
    # repli de familles CSS disparait. Voir tools/ladybird/fontconfig/fonts.conf.
    # =========================================================================

    $FontconfigTarget = Join-Path $LadybirdShare "fontconfig"

    if (-not (Test-Path (Join-Path $FontconfigTarget "fonts.conf"))) {

        $FontconfigSource = Join-Path `
            $RepoRoot `
            "tools\ladybird\fontconfig\fonts.conf"

        if (Test-Path $FontconfigSource) {

            New-Item `
                -ItemType Directory `
                -Path $FontconfigTarget `
                -Force | Out-Null

            Copy-Item `
                $FontconfigSource `
                (Join-Path $FontconfigTarget "fonts.conf") `
                -Force
        }
    }


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
        # `BO_AUTOSTART_BROWSER` n'est pose que par les regressions de CI, qui
        # n'ont pas de main pour cliquer. Une session normale arrive sur le
        # bureau et attend : le navigateur s'ouvre au double-clic sur l'icone
        # "Navigateur". Un systeme qui ouvre un navigateur tout seul au
        # demarrage n'est pas un systeme, c'est une demonstration.
        #
        # Les variables restent exportees dans les deux cas : le gestionnaire de
        # fenetres transmet l'environnement du shell au client qu'il lance
        # (`kernel::exec::shell_environment`), donc le navigateur lance a la
        # souris recoit exactement la meme configuration.
        $lignesScenario = if ($IsLadybirdM9Test) {
            @(
                'echo "=== Ladybird M9 : HTTP distant via RequestServer ==="',
                'export BO_AUTOSTART_BROWSER=1',
                'export BOUCHAUD_M9_TEST=1'
            )
        }
        else {
            @(
                'echo "=== Bouchaud OS : bureau, navigateur au double-clic ==="',
                'echo "Navigateur : double-clic sur l icone, ou menu Demarrer"'
            )
        }

        $chromeLine = if ($LadybirdChrome) { 'export BOUCHAUD_M11=1' } else { 'echo "M11 desactive : capture unique, sans entrees"' }
        $hostLine = if ($LadybirdInteractif) { 'export BOUCHAUD_BROWSER_HOST=1' } else { 'echo "Browser Host desactive : regression M9"' }
        $timezoneLine = if ($LadybirdInteractif) { 'export BOUCHAUD_TIME_ZONE=Europe/Paris' } else { 'echo "Timezone Browser Host inactive"' }
        $popupLine = if ($LadybirdInteractif) { 'export BOUCHAUD_ALLOW_POPUPS=1' } else { 'echo "Popups Browser Host inactifs"' }

        $autorun = @(
            @('uname', 'df') +
            $lignesScenario +
            @(
                'export BOUCHAUD_M9=1',
                "export BOUCHAUD_M9_URL=$(ConvertTo-ShellSingleQuoted $LadybirdUrl)",
                $chromeLine,
                $hostLine,
                $timezoneLine,
                $popupLine,
                'desktop',
                ''
            )
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


# Le bit d'execution ne survit pas a un checkout Windows : cette liste le
# repose au moment d'archiver. Tout binaire ajoute au scenario doit y figurer,
# sinon Bouchaud refuse de l'executer.
#
# WebWorker n'y est pas : il n'est pas encore lance faute de processus hote
# capable de repondre a `StartWorkerAgent`.
executables = {
    "bo-navigateur",
    "usr/libexec/ladybird/BouchaudBrowserHost",
    "usr/libexec/ladybird/WebContent",
    "usr/libexec/ladybird/RequestServer",
    "usr/libexec/ladybird/ImageDecoder",
    "usr/libexec/ladybird/Compositor",
    "usr/libexec/ladybird/WebWorker",
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
        "$RamMiB"
    )

    $qemuArgs += @(
        "-smp",
        "$CpuCount"
    )

    if ($CpuCount -gt 1) {
        Write-Host `
            "ATTENTION: $CpuCount vCPU exposes. Le noyau ne demarre aucun processeur applicatif : ni ACPI/MADT, ni LAPIC, ni INIT/SIPI. Les autres resteront eteints." `
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
        $ServiceLabel = "WebContent + ImageDecoder (regression locale)"
        $BrowserChain = "/bo-navigateur -> bootstrap -> ImageDecoder + WebContent"
    }
    elseif ($IsLadybirdM9Test) {
        $ModeLabel = "Ladybird natif M9 TEST HTTP"
        $ServiceLabel = "WebContent + RequestServer + ImageDecoder"
        $BrowserChain = "/bo-navigateur -> bootstrap -> ImageDecoder + RequestServer + WebContent"
    }
    elseif ($LadybirdChrome) {
        $ModeLabel = "Bouchaud Navigateur (Ladybird M11)"
        $ServiceLabel = "WebContent + RequestServer + ImageDecoder"
        $BrowserChain = "/bo-navigateur -> bootstrap -> ImageDecoder + RequestServer + WebContent + chrome"
    }
    else {
        $ModeLabel = "Ladybird natif M9 interactif (sans chrome)"
        $ServiceLabel = "WebContent + RequestServer + ImageDecoder"
        $BrowserChain = "/bo-navigateur -> bootstrap -> ImageDecoder + RequestServer + WebContent"
    }

    Write-Host `
        "mode       : $ModeLabel" `
        -ForegroundColor Green

    Write-Host `
        "RAM        : $RamMiB Mio"

    Write-Host `
        "vCPU       : $CpuCount (le noyau n ordonnance que sur le BSP)"

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
