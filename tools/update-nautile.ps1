# Synchronise le code source de Nautile/Bouchaud OS avant de regenerer l'image.
# Nautile est compile dans le noyau depuis src/browser/* : il doit donc etre
# mis a jour cote hote AVANT `cargo bootimage`, pas depuis l'OS deja boote.

param(
  [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot ".."))
)

$ErrorActionPreference = "Stop"

function Invoke-GitChecked {
  param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Args)
  & git @Args
  if ($LASTEXITCODE -ne 0) {
    throw "git $($Args -join ' ') a echoue (code $LASTEXITCODE)"
  }
}

try {
  Write-Host "=== Mise a jour Nautile depuis Git ===" -ForegroundColor Cyan

  if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw "git introuvable : impossible de mettre Nautile a jour automatiquement."
  }

  Invoke-GitChecked -C $RepoRoot rev-parse --is-inside-work-tree | Out-Null

  $branch = (& git -C $RepoRoot rev-parse --abbrev-ref HEAD).Trim()
  if ($LASTEXITCODE -ne 0) {
    throw "Impossible de determiner la branche Git courante."
  }

  if ($branch -eq "HEAD") {
    throw "Depot en HEAD detache : impossible de savoir quelle branche mettre a jour."
  }

  $upstream = (& git -C $RepoRoot rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>$null).Trim()
  if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($upstream)) {
    throw "La branche '$branch' n'a pas d'upstream Git. Configure-la avec : git branch --set-upstream-to=origin/$branch $branch"
  }

  Write-Host "Branche : $branch -> $upstream" -ForegroundColor DarkCyan
  Invoke-GitChecked -C $RepoRoot fetch --prune
  Invoke-GitChecked -C $RepoRoot pull --ff-only

  $nautileMerge = (& git -C $RepoRoot log --merges -1 --date=short --format="%h %cd %s" -- src/browser).Trim()
  if ([string]::IsNullOrWhiteSpace($nautileMerge)) {
    $nautileMerge = (& git -C $RepoRoot log -1 --date=short --format="%h %cd %s" -- src/browser).Trim()
  }
  $nautileSource = (& git -C $RepoRoot log -1 --date=short --format="%h %cd %s" -- src/browser).Trim()

  Write-Host "Nautile/Bouchaud OS est a jour avant bootimage." -ForegroundColor Green
  Write-Host "Nautile dernier merge compile : $nautileMerge" -ForegroundColor Green
  Write-Host "Nautile dernier changement source : $nautileSource" -ForegroundColor DarkGreen
  Write-Host "Ces references seront injectees dans la banniere de boot par build.rs." -ForegroundColor DarkCyan
  exit 0
} catch {
  Write-Host $_.Exception.Message -ForegroundColor Red
  exit 1
}
