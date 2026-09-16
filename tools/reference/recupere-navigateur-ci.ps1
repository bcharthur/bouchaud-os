# Recupere les binaires Ladybird deja construits par la CI, au lieu de les
# reconstruire.
#
#   .\tools\reference\recupere-navigateur-ci.ps1
#   .\tools\reference\recupere-navigateur-ci.ps1 -RunId 34865168564
#
# ## Pourquoi ce script existe
#
# `prepare-reference-ladybird.ps1` exige `native-browser-m9\`, c'est-a-dire les
# binaires du navigateur : BouchaudBrowserHost, WebContent, RequestServer,
# ImageDecoder, Compositor, WebWorker, WebDriver. Ils ne sont pas dans le depot
# -- ce sont des artefacts de construction, pas des sources.
#
# Sans eux, la seule voie documentee etait `.\run.ps1 -Ladybird`, c'est-a-dire
# recuperer l'arbre Ladybird epingle, construire vcpkg (Skia, ICU, HarfBuzz...)
# puis LibWeb et les services. Sur une machine a quatre coeurs, cela se compte
# en heures, et il faut plusieurs dizaines de gibioctets.
#
# Or la CI le fait deja a chaque changement : le workflow
# `ladybird-native-browser.yml` publie l'artefact
# `bouchaud-ladybird-native-browser` -- le contenu de
# `third_party/native-browser-bouchaud/`, soit exactement ce qu'attend
# `native-browser-m9`. Environ quatre cent trente mebioctets, garde quatorze
# jours.
#
# Telecharger quatre cent trente mebioctets plutot que reconstruire pendant
# quatre heures n'est pas un raccourci : c'est l'artefact que la CI a
# VERIFIE, au SHA qu'elle a teste.
#
# ## Ce que ce script ne fait pas
#
# Il ne construit rien et ne verifie pas le contenu des binaires : c'est le
# travail de `prepare-reference-ladybird.ps1`, qui tourne juste apres et qui
# refuse un artefact incomplet ou d'une ancienne generation. Ce script ne fait
# que poser les fichiers au bon endroit.

param(
    # Identifiant d'une execution de `ladybird-native-browser.yml`. Par defaut,
    # la derniere reussie sur n'importe quelle branche.
    [string]$RunId = "",

    # Reprendre meme si native-browser-m9 existe deja.
    [switch]$Force
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot

function Fail([string]$Message) {
    Write-Host ""
    Write-Host "ERREUR: $Message" -ForegroundColor Red
    exit 1
}

$Artefact = "bouchaud-ladybird-native-browser"
$Cible = Join-Path $RepoRoot "native-browser-m9"

Write-Host "=== Recuperation des binaires Ladybird depuis la CI ===" -ForegroundColor Cyan

if ((Test-Path -LiteralPath $Cible -PathType Container) -and -not $Force) {
    Write-Host "native-browser-m9 existe deja. -Force pour le remplacer." -ForegroundColor Yellow
    Write-Host "Suite : .\tools\reference\run-reference-stage2-ladybird.ps1"
    exit 0
}

$Gh = Get-Command gh -ErrorAction SilentlyContinue
if (-not $Gh) {
    Fail (
        "gh introuvable. Deux voies :`n" +
        "  1. winget install --id GitHub.cli   puis  gh auth login`n" +
        "  2. a la main : ouvrir la page d'execution du workflow " +
        "ladybird-native-browser sur GitHub, telecharger l'artefact " +
        "$Artefact, et decompresser son contenu dans native-browser-m9\ " +
        "(les fichiers a la racine, pas dans un sous-dossier)."
    )
}

if (-not $RunId) {
    Write-Host "Recherche de la derniere execution reussie..." -ForegroundColor Yellow
    $RunId = & gh run list `
        --workflow ladybird-native-browser.yml `
        --status success `
        --limit 1 `
        --json databaseId `
        --jq '.[0].databaseId' 2>$null
    if ($LASTEXITCODE -ne 0 -or -not $RunId) {
        Fail "aucune execution reussie trouvee. Preciser -RunId <id>."
    }
}

Write-Host "Execution : $RunId" -ForegroundColor Green
Write-Host "Artefact  : $Artefact (environ 430 Mio)"
Write-Host ""

if (Test-Path -LiteralPath $Cible) {
    Remove-Item -LiteralPath $Cible -Recurse -Force
}
New-Item -ItemType Directory -Path $Cible -Force | Out-Null

& gh run download $RunId --name $Artefact --dir $Cible
if ($LASTEXITCODE -ne 0) {
    Fail (
        "telechargement en echec. L'artefact n'est garde que QUATORZE JOURS : " +
        "si cette execution est plus ancienne, en relancer une (gh workflow " +
        "run ladybird-native-browser.yml) ou choisir une execution plus " +
        "recente avec -RunId."
    )
}

# Un `upload-artifact` sur un repertoire peut poser son contenu a la racine ou
# dans un sous-dossier selon la version. On aplatit le second cas plutot que de
# laisser `prepare-reference-ladybird.ps1` echouer sur un fichier absent qui
# est en realite un niveau plus bas.
$Repere = Join-Path $Cible "BouchaudBrowserHost"
if (-not (Test-Path -LiteralPath $Repere)) {
    $Interieur = Get-ChildItem -LiteralPath $Cible -Directory |
        Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName "BouchaudBrowserHost") } |
        Select-Object -First 1
    if ($Interieur) {
        Write-Host "Contenu trouve dans $($Interieur.Name), aplatissement." -ForegroundColor Yellow
        Get-ChildItem -LiteralPath $Interieur.FullName -Force |
            Move-Item -Destination $Cible -Force
        Remove-Item -LiteralPath $Interieur.FullName -Recurse -Force
    }
}

$Manquants = @()
foreach ($Nom in @(
    "BouchaudBrowserHost", "WebContent", "RequestServer", "ImageDecoder",
    "Compositor", "WebWorker", "WebDriver", "webcontent-bootstrap",
    "M9_CAPABLE", "V16_UI_CAPABLE", "V19_UI_CAPABLE", "resources"
)) {
    if (-not (Test-Path -LiteralPath (Join-Path $Cible $Nom))) {
        $Manquants += $Nom
    }
}

if ($Manquants.Count -gt 0) {
    Fail (
        "artefact incomplet, absents : " + ($Manquants -join ", ") + "`n" +
        "L'execution $RunId n'a peut-etre pas construit la meme generation. " +
        "Essayer une execution plus recente avec -RunId."
    )
}

Write-Host ""
Write-Host "native-browser-m9 pret." -ForegroundColor Green
Write-Host "Suite :"
Write-Host "  .\tools\reference\run-reference-stage2-ladybird.ps1"
Write-Host ""
Write-Host "Ou, pour la forme complete de la machine de reference :"
Write-Host "  .\tools\reference\run-reference-stage2-ladybird.ps1 -RamMiB 12288"
