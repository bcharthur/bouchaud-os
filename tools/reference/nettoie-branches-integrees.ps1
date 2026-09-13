# Supprime uniquement les quatre branches consolidees le 13 septembre 2026.
# Chaque suppression exige l'ascendance dans main et le SHA distant attendu.
[CmdletBinding()]
param([switch]$Apercu)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot
& git fetch origin --prune
if ($LASTEXITCODE -ne 0) { throw "git fetch a echoue" }

$Branches = @(
    "feat/trigkey-reference-hardware",
    "fix/trigkey-smp-double-fault-stack-safe-gpt",
    "fix2",
    "temp"
)
foreach ($Branche in $Branches) {
    & git show-ref --verify --quiet "refs/remotes/origin/$Branche"
    if ($LASTEXITCODE -eq 1) {
        Write-Host "Deja absente : $Branche"
        continue
    }
    if ($LASTEXITCODE -ne 0) { throw "Lecture de la reference impossible : $Branche" }
    $Sha = & git rev-parse --verify "refs/remotes/origin/$Branche"
    if ($LASTEXITCODE -ne 0) { throw "SHA introuvable : $Branche" }
    $Sha = $Sha.Trim()
    & git merge-base --is-ancestor $Sha origin/main
    if ($LASTEXITCODE -ne 0) { throw "Travail non integre dans main : $Branche. Suppression refusee." }
    if ($Apercu) {
        Write-Host "Integree et supprimable : $Branche ($Sha)"
        continue
    }
    & git push origin "--force-with-lease=refs/heads/${Branche}:$Sha" ":refs/heads/$Branche"
    if ($LASTEXITCODE -ne 0) { throw "Suppression refusee pour $Branche (droits ou branche modifiee)." }
}
