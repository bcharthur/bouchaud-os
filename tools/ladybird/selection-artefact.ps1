# Selection du run GitHub Actions qui porte un artefact Ladybird utilisable.
#
# POURQUOI CE FICHIER EXISTE SEPAREMENT DE run.ps1
# ------------------------------------------------
# `run.ps1` s'execute de bout en bout des qu'on le charge : construction du
# noyau, fabrication du disque, lancement de QEMU. Aucun test ne peut donc en
# appeler une partie. La CI ne faisait qu'ANALYSER le fichier -- elle verifiait
# qu'il se parse, jamais qu'il fonctionne.
#
# Trois defauts d'execution sont passes par ce trou, tous dans ce bloc-ci :
#
#   * un guillemet echappe a la mode du C (`\"`), qui coupait la chaine ;
#   * un programme jq dont Windows PowerShell 5.1 mangeait les guillemets,
#     d'ou un "function not defined: browser/0" venu de jq ;
#   * un `.Trim()` sur un identifiant de run devenu Int64 apres le passage a
#     `ConvertFrom-Json`.
#
# Aucun n'etait visible a l'analyse : le fichier se parsait parfaitement a
# chaque fois. Il fallait EXECUTER. C'est ce que `tools/ci/test-selection-
# artefact.ps1` fait maintenant, avec un `gh` simule, sous
# `$ErrorActionPreference = "Stop"`.
#
# CE QUE LA FONCTION DECIDE
# -------------------------
# Le critere est l'ARTEFACT LUI-MEME, jamais la conclusion du run.
# `ladybird-native-browser.yml` a deux jobs et un seul fabrique quelque
# chose : "build once" televerse, "browser-host smoke" telecharge. Un
# consommateur rouge ne doit pas cacher un producteur sain.
#
# Le critere subsume l'ancien -- `if-no-files-found: error` garantit que
# l'artefact n'existe que si le producteur a abouti -- et il ajoute ce que
# l'ancien ignorait : la retention. Un run vert de plus de quatorze jours
# etait retenu, puis le telechargement echouait sans dire pourquoi.

Set-StrictMode -Version Latest

function Get-RunAvecArtefact {

    [CmdletBinding()]
    param(
        [Parameter(Mandatory)]
        [string]$Branche,

        [Parameter(Mandatory)]
        [string]$Artefact,

        [string]$Workflow = "ladybird-native-browser.yml",

        [int]$Limite = 20
    )

    # Le filtrage se fait en PowerShell, jamais par `--jq`.
    #
    # Windows PowerShell 5.1 reconstruit une LIGNE DE COMMANDE pour lancer un
    # programme natif, et mange les guillemets doubles contenus dans un
    # argument. Un programme jq comme
    #
    #     select(.name == "bouchaud-ladybird-native-browser")
    #
    # arrivait a `gh` sans ses guillemets, et jq lisait
    # `bouchaud - ladybird - native - browser` : trois soustractions et un
    # appel a une fonction inexistante. `ConvertFrom-Json` supprime la classe
    # entiere -- plus aucun guillemet ne traverse la frontiere des arguments.
    $runsBrut = (& gh run list `
        --workflow $Workflow `
        --branch $Branche `
        --limit $Limite `
        --json databaseId) -join "`n"

    $identifiants = @()

    if ($LASTEXITCODE -eq 0 -and $runsBrut.Trim()) {

        $identifiants = @(
            (ConvertFrom-Json $runsBrut) | ForEach-Object { $_.databaseId }
        )
    }

    $examines = 0

    foreach ($candidat in $identifiants) {

        # Pas de `.Trim()` ici : `ConvertFrom-Json` rend un Int64, pas une
        # ligne de texte. C'est le troisieme defaut cite en tete de fichier.
        $examines += 1

        # `expired` est le seul champ qui distingue un artefact encore
        # telechargeable d'une simple trace dans l'historique.
        $brut = (& gh api `
            "repos/{owner}/{repo}/actions/runs/$candidat/artifacts" `
            2>$null) -join "`n"

        if ($LASTEXITCODE -ne 0 -or -not $brut.Trim()) {
            continue
        }

        $vivants = @(
            (ConvertFrom-Json $brut).artifacts |
                Where-Object { $_.name -eq $Artefact -and -not $_.expired }
        ).Count

        if ($vivants -gt 0) {

            return [pscustomobject]@{
                RunId    = [long]$candidat
                Examines = $examines
            }
        }
    }

    return [pscustomobject]@{
        RunId    = $null
        Examines = $examines
    }
}
