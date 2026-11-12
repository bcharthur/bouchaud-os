<#
.SYNOPSIS
  BOUCHAUD_WINDOWS_V1 -- construit l'image de LIVRAISON du Trigkey depuis Windows.

.DESCRIPTION
  Orchestration seulement : l'image est construite sous Linux (WSL 2 sous
  Windows, bash sous Linux) par tools/release/build_trigkey_ab_image.sh, la
  MEME commande que la CI et que le workflow de release.

  1. SHA Git exact ; refus d'un arbre sale.
  2. Ladybird : -Ladybird DOSSIER, sinon recuperation de l'artefact CI par
     tools/reference/recupere-navigateur-ci.ps1 (exige `gh` authentifie).
     -Developpement : image SANS Ladybird (non livrable, et dite comme telle).
  3. Construction (aucune ecriture de peripherique).
  4. Verification : empreinte de disque-ab.img, composants, livrable=oui
     exige sauf -Developpement.
  Journal : target\logs\build-*.log.

  Prerequis WSL (une fois) : Ubuntu avec rustup (toolchain du depot),
  musl-tools, build-essential, binutils, python3, git.

.EXAMPLE
  .\tools\windows\Build-BouchaudOS.ps1
  .\tools\windows\Build-BouchaudOS.ps1 -Developpement
#>
[CmdletBinding()]
param(
    [string]$Sortie = 'target/livraison',
    [string]$Ladybird = '',
    [switch]$Developpement
)
$ErrorActionPreference = 'Stop'
Import-Module (Join-Path $PSScriptRoot 'BouchaudWindows.psm1') -Force
$journal = Start-JournalBouchaud 'build'
try {
    $racine = Get-RacineDepot
    $sha = Get-ShaGit
    Write-Etape "commit $sha"

    $sortiePleine = Join-Path $racine $Sortie
    if (Test-Path -LiteralPath (Join-Path $sortiePleine 'disque-ab.img')) {
        Stop-Bouchaud "$Sortie contient deja une image : la deplacer ou choisir -Sortie (aucun ecrasement)"
    }

    $envLadybird = ''
    if (-not $Developpement) {
        if (-not $Ladybird) {
            $Ladybird = Join-Path $racine 'native-browser-m9'
            if (-not (Test-Path -LiteralPath (Join-Path $Ladybird 'BouchaudBrowserHost'))) {
                Write-Etape 'artefact Ladybird absent : recuperation depuis la CI'
                & (Join-Path $racine 'tools/reference/recupere-navigateur-ci.ps1')
                if ($LASTEXITCODE -ne 0) { Stop-Bouchaud 'recuperation de l artefact Ladybird en echec' }
            }
        }
        if (-not (Test-Path -LiteralPath (Join-Path $Ladybird 'BouchaudBrowserHost'))) {
            Stop-Bouchaud "artefact Ladybird incomplet dans $Ladybird"
        }
        $envLadybird = "BO_NATIF_LADYBIRD='$(ConvertTo-CheminLinux $Ladybird)' "
    } else {
        Write-Host 'ATTENTION : -Developpement, image SANS Ladybird (non livrable).' -ForegroundColor Yellow
    }

    Write-Etape 'construction (Linux)'
    $code = Invoke-Linux ("{0}bash tools/release/build_trigkey_ab_image.sh '{1}'" -f $envLadybird, (ConvertTo-CheminLinux $sortiePleine))
    if ($code -ne 0) { Stop-Bouchaud "construction en echec (code $code)" }

    $disque = Join-Path $sortiePleine 'disque-ab.img'
    $empreinte = Assert-Empreinte $disque
    $c = Get-Composants $sortiePleine
    Write-Ok "image $disque sha256=$empreinte"
    Write-Ok ("composants : ladybird={0} doom={1} livrable={2}" -f $c['ladybird'], $c['doom'], $c['livrable'])
    if ($c['livrable'] -ne 'oui' -and -not $Developpement) { Stop-Bouchaud 'image non livrable' }
    Write-Host "BOUCHAUD_BUILD_WINDOWS_OK commit=$sha sha256=$empreinte livrable=$($c['livrable']) journal=$journal"
} finally {
    Stop-Transcript | Out-Null
}
