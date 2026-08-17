param(
    [switch]$Fullscreen,

    # Lance le vrai Ladybird/WebContent natif au lieu du navigateur Qt/Python
    # du userland classique.
    #
    # Usage :
    #   .\run.ps1 -Ladybird
    [switch]$Ladybird,

    # Force le retelechargement de l'artefact Ladybird depuis GitHub Actions.
    [switch]$RefreshLadybird,

    # Run GitHub Actions M8 valide contenant WebContent natif Bouchaud.
    #
    # Peut etre surcharge :
    #   .\run.ps1 -Ladybird -LadybirdRunId 123456789
    [long]$LadybirdRunId = 32037726275,

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

if ($Ladybird) {

    Write-Section "Ladybird natif"

    Ensure-Python


    $NativeBrowserDir = Join-Path `
        $RepoRoot `
        "native-browser-m8"


    $LadybirdImage = Join-Path `
        $RepoRoot `
        "ladybird-browser.img"


    $ScenarioDir = Join-Path `
        $RepoRoot `
        "scenario-m8"


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

    $RequiredLadybirdFiles = @(
        "WebContent",
        "webcontent-bootstrap"
    )


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


        Write-Host (
            "Ladybird : telechargement artefact du run {0}" -f `
                $LadybirdRunId
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


        gh run download $LadybirdRunId `
            -n "bouchaud-ladybird-native-browser" `
            -D $NativeBrowserDir


        if ($LASTEXITCODE -ne 0) {

            Fail "impossible de telecharger l'artefact Ladybird du run $LadybirdRunId"
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
    # Autorun
    # =========================================================================

    $AutorunPath = Join-Path `
        $ScenarioDir `
        "autorun"


    # LF uniquement.
    #
    # Pas de CRLF Windows dans le fichier execute dans Bouchaud OS.
    $autorun = @(
        'uname',
        'df',
        'echo "=== Ladybird M8 : HTML local dans fenetre Bouchaud ==="',
        'export BO_AUTOSTART_BROWSER=1',
        'export BOUCHAUD_M8=1',
        'desktop',
        ''
    ) -join "`n"


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

if ($Ladybird) {

    # M8 CI utilise 4 Gio.
    $qemuArgs += @(
        "-m",
        "4096"
    )
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


if ($Ladybird) {

    Write-Host `
        "mode       : Ladybird natif M8" `
        -ForegroundColor Green

    Write-Host `
        "RAM        : 4096 Mio"

    Write-Host `
        "navigateur : /bo-navigateur -> webcontent-bootstrap -> WebContent"

    Write-Host `
        "services   : WebContent uniquement (M8 minimal)"

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


& $QemuExe @qemuArgs


$QemuExitCode = $LASTEXITCODE


Write-Host ""

Write-Host (
    "QEMU termine avec le code {0}" -f `
        $QemuExitCode
) -ForegroundColor DarkGray


exit $QemuExitCode