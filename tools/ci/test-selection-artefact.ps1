# Execute la selection d'artefact Ladybird avec un `gh` simule.
#
# POURQUOI CE TEST EXISTE
# -----------------------
# La CI ANALYSAIT `run.ps1` sans jamais l'executer. Trois defauts sont passes
# par ce trou, tous dans le meme bloc, et tous invisibles a l'analyse :
#
#   * `\"` pour echapper un guillemet -- PowerShell n'a pas d'echappement par
#     barre oblique inverse, la chaine se terminait au milieu ;
#   * un programme jq dont Windows PowerShell 5.1 mangeait les guillemets ;
#   * un `.Trim()` sur un identifiant devenu Int64.
#
# Le fichier se parsait parfaitement dans les trois cas.
#
# CE QUE CE TEST EXIGE, ET QUI EST L'ESSENTIEL
# --------------------------------------------
# Pas seulement que le bon run soit retenu : que le FLUX D'ERREUR SOIT VIDE.
#
# C'est la lecon du troisieme defaut. Il s'est produit pendant un banc d'essai
# qui cherchait la ligne de succes par `grep` -- et qui a donc FILTRE le
# message d'erreur au lieu de le voir. Un `.Trim()` qui echoue est une erreur
# non terminante : PowerShell la signale, laisse la variable inchangee, et
# continue. Le test passait pendant que l'utilisateur voyait une pile rouge.
#
# `$ErrorActionPreference = "Stop"` transforme toute la classe -- methode
# absente, propriete absente, conversion impossible -- en echec franc.
#
# `gh` est remplace par une FONCTION, pas par un fichier sur le PATH : une
# fonction PowerShell masque une commande externe du meme nom, et ce mecanisme
# se comporte identiquement sur Windows et sur Linux.
#
# CE QUE CE TEST NE PEUT PAS VOIR
# -------------------------------
# Le massacre des guillemets par Windows PowerShell 5.1. Il n'a lieu qu'en
# lancant un vrai programme EXTERNE, quand PowerShell reconstruit une ligne de
# commande ; une fonction recoit ses arguments deja decoupes. Un `--jq` avec
# guillemets passerait donc ce test sans broncher.
#
# C'est le travail de `tools/verifie-artefact-navigateur.py`, qui interdit
# `--jq` par lecture du source. Les deux mecanismes sont complementaires et
# aucun ne remplace l'autre : le verificateur statique couvre le passage des
# arguments, ce test couvre les erreurs de type et de methode.

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Racine = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
. (Join-Path $Racine "tools/ladybird/selection-artefact.ps1")

$ARTEFACT = "bouchaud-ladybird-native-browser"

# Etat du `gh` simule, pose par chaque cas.
$script:Runs = @()
$script:Vivants = @()
$script:Expires = @()

function gh {

    $arguments = $args

    if ($arguments[0] -eq "run" -and $arguments[1] -eq "list") {

        $global:LASTEXITCODE = 0

        return (
            "[" + (($script:Runs | ForEach-Object {
                '{"databaseId":' + $_ + '}'
            }) -join ",") + "]"
        )
    }

    if ($arguments[0] -eq "api") {

        $global:LASTEXITCODE = 0

        $chemin = $arguments[1]
        $identifiant = ($chemin -replace '.*/runs/', '') -replace '/artifacts.*', ''

        if ($script:Vivants -contains [long]$identifiant) {
            return '{"artifacts":[' +
                '{"name":"ladybird-browser-host-smoke-logs","expired":false},' +
                '{"name":"' + $ARTEFACT + '","expired":false}]}'
        }

        if ($script:Expires -contains [long]$identifiant) {
            return '{"artifacts":[{"name":"' + $ARTEFACT + '","expired":true}]}'
        }

        return '{"artifacts":[]}'
    }

    throw "gh simule : appel inattendu -- $arguments"
}

$echecs = 0

function Verifie {

    param(
        [string]$Cas,
        [long[]]$Runs,
        [long[]]$Vivants,
        [long[]]$Expires,
        $RunAttendu,
        [int]$ExaminesAttendus
    )

    $script:Runs = $Runs
    $script:Vivants = $Vivants
    $script:Expires = $Expires

    $erreurs = @()
    $obtenu = $null

    try {
        $obtenu = Get-RunAvecArtefact -Branche "main" -Artefact $ARTEFACT -ErrorVariable erreurs
    }
    catch {
        Write-Host ("  ECHEC  {0}" -f $Cas) -ForegroundColor Red
        Write-Host ("         exception : {0}" -f $_.Exception.Message)
        $script:echecs += 1
        return
    }

    # LE controle qui manquait : rien ne doit avoir ete ecrit sur le flux
    # d'erreur, meme si la fonction a rendu la bonne valeur.
    if ($erreurs.Count -gt 0) {
        Write-Host ("  ECHEC  {0}" -f $Cas) -ForegroundColor Red
        foreach ($erreur in $erreurs) {
            Write-Host ("         flux d'erreur : {0}" -f $erreur)
        }
        $script:echecs += 1
        return
    }

    if ($obtenu.RunId -ne $RunAttendu) {
        Write-Host ("  ECHEC  {0}" -f $Cas) -ForegroundColor Red
        Write-Host ("         run attendu {0}, obtenu {1}" -f $RunAttendu, $obtenu.RunId)
        $script:echecs += 1
        return
    }

    if ($obtenu.Examines -ne $ExaminesAttendus) {
        Write-Host ("  ECHEC  {0}" -f $Cas) -ForegroundColor Red
        Write-Host ("         {0} run(s) examines, {1} attendus" -f `
            $obtenu.Examines, $ExaminesAttendus)
        $script:echecs += 1
        return
    }

    Write-Host ("  ok     {0}" -f $Cas) -ForegroundColor Green
}

Write-Host "== selection d'artefact Ladybird (gh simule) =="

Verifie -Cas "le run le plus recent porte un artefact vivant" `
    -Runs @(900, 899, 898) -Vivants @(900, 899) -Expires @() `
    -RunAttendu 900 -ExaminesAttendus 1

# Le cas pour lequel tout ce code existe : le run le plus recent est rouge
# (son smoke a echoue) mais son artefact est la. La conclusion du run n'entre
# jamais dans la decision, seul l'artefact compte.
Verifie -Cas "un run rouge dont l'artefact vit est retenu quand meme" `
    -Runs @(900, 899) -Vivants @(900) -Expires @() `
    -RunAttendu 900 -ExaminesAttendus 1

Verifie -Cas "les deux plus recents ont EXPIRE, le troisieme vit" `
    -Runs @(900, 899, 898) -Vivants @(898) -Expires @(900, 899) `
    -RunAttendu 898 -ExaminesAttendus 3

Verifie -Cas "aucun artefact vivant" `
    -Runs @(900, 899) -Vivants @() -Expires @(900) `
    -RunAttendu $null -ExaminesAttendus 2

Verifie -Cas "aucun run sur la branche" `
    -Runs @() -Vivants @() -Expires @() `
    -RunAttendu $null -ExaminesAttendus 0

Write-Host ""

if ($echecs -gt 0) {
    Write-Host ("selection d'artefact : {0} cas en echec" -f $echecs) -ForegroundColor Red
    exit 1
}

Write-Host "selection d'artefact : tous les cas passent, flux d'erreur vide" `
    -ForegroundColor Green
exit 0
