# Ce que l'on s'apprete a construire, en une ligne.
#
#   .\tools\etat-source.ps1 [-Sync]
#
# `run.ps1` l'appelle avant `cargo bootimage`, pour que la
# console dise quel commit part dans l'image. Il ne **bloque jamais** le
# demarrage : un depot sans upstream, une machine hors ligne ou un `git`
# introuvable donnent un avertissement, pas un refus de booter.
#
# ## Ce qu'il remplace, et pourquoi
#
# `tools/update-nautile.ps1` faisait la meme chose pour Nautile, le moteur web
# qui vivait dans le noyau sous `src/browser/`. Ce moteur a ete sorti du ring 0
# le 2026-08-04 (commit b2d2615) et reecrit en ring 3 : le repertoire n'existe
# plus, et le `build.rs` dans lequel le script disait injecter ses references
# n'a jamais existe non plus. Le script interrogeait donc un sous-systeme
# disparu - et comme `run.ps1` s'arretait sur son code de sortie, il empechait
# purement et simplement de demarrer l'OS.
#
# Il tombait sur un second piege, qui vaut d'etre garde en memoire : il passait
# un chemin a git sous la forme `git log ... -- src/browser`, depuis une
# fonction PowerShell a `ValueFromRemainingArguments`. **Le lieur de parametres
# de PowerShell consomme le `--`** - c'est son marqueur de fin d'options - et
# git recevait `src/browser` comme une revision, d'ou le "ambiguous argument"
# et le code 128. Un pathspec passe a git depuis une fonction PowerShell doit
# donc l'etre autrement : `--` seul ne survit pas au trajet. Rien ici n'en
# utilise, ce qui est la facon la plus sure de ne pas s'y reprendre.

param(
  [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")),
  # Met a jour la branche courante avant de construire. **Volontairement pas le
  # defaut** : tirer automatiquement a chaque demarrage change le code qu'on
  # s'apprete a booter sans le dire, ce qui transforme "je relance pour
  # reproduire" en "je relance sur autre chose".
  [switch]$Sync
)

function Avertis($texte) {
  Write-Host $texte -ForegroundColor DarkYellow
}

Write-Host "=== Source ===" -ForegroundColor Cyan

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
  Avertis "git introuvable : etat de la source inconnu."
  exit 0
}

& git -C $RepoRoot rev-parse --is-inside-work-tree 2>$null | Out-Null
if ($LASTEXITCODE -ne 0) {
  Avertis "pas un depot git : etat de la source inconnu."
  exit 0
}

$branche = (& git -C $RepoRoot rev-parse --abbrev-ref HEAD | Out-String).Trim()

if ($Sync) {
  $upstream = (& git -C $RepoRoot rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>$null | Out-String).Trim()
  if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($upstream)) {
    Avertis "la branche '$branche' n'a pas d'upstream : rien a synchroniser."
  } else {
    Write-Host "synchronisation : $branche -> $upstream" -ForegroundColor DarkCyan
    & git -C $RepoRoot fetch --prune
    if ($LASTEXITCODE -ne 0) {
      Avertis "fetch impossible (hors ligne ?) : on construit ce qui est la."
    } else {
      & git -C $RepoRoot pull --ff-only
      if ($LASTEXITCODE -ne 0) {
        Avertis "pull --ff-only refuse : la branche a diverge. On construit ce qui est la."
      }
    }
  }
}

$commit = (& git -C $RepoRoot log -1 --date=short --format="%h %cd %s" | Out-String).Trim()
$sale = (& git -C $RepoRoot status --porcelain | Out-String).Trim()

Write-Host "branche : $branche" -ForegroundColor Green
Write-Host "commit  : $commit" -ForegroundColor Green
if (-not [string]::IsNullOrWhiteSpace($sale)) {
  $combien = ($sale -split "`n").Count
  Avertis "arbre modifie : $combien fichier(s) non commite(s) partent dans l'image."
}

# Le navigateur ne vit plus dans le noyau : il est dans le disque userland, que
# `tools/userland/mkdisk.sh` fabrique. Une image absente n'empeche pas de
# booter, elle donne un OS sans userland - et c'est la confusion la plus
# frequente, donc elle est dite ici.
$userland = Join-Path $RepoRoot "tools\userland\userland.img"
if (Test-Path $userland) {
  $taille = [math]::Round((Get-Item $userland).Length / 1MB, 1)
  Write-Host "userland: $userland ($taille Mo)" -ForegroundColor Green
} else {
  Avertis "pas de disque userland : ni Qt, ni Python, ni navigateur dans l'OS."
  Avertis "  le fabriquer avec tools/userland/mkdisk.sh (sous WSL ou Linux)."
}

exit 0
