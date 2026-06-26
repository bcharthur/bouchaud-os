# Synchronise le code source de Nautile/Bouchaud OS avant de regenerer l'image.
# Nautile est compile dans le noyau depuis src/browser/* : il doit donc etre
# mis a jour cote hote AVANT `cargo bootimage`, pas depuis l'OS deja boote.

param(
  [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")),
  [string]$NautileRepoUrl = "https://github.com/bcharthur/nautile-navigateur.git",
  [string]$NautileBranch = "main"
)

$ErrorActionPreference = "Stop"

function Invoke-GitChecked {
  param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Args)
  & git @Args
  if ($LASTEXITCODE -ne 0) {
    throw "git $($Args -join ' ') a echoue (code $LASTEXITCODE)"
  }
}

function Invoke-GitText {
  param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Args)
  $output = & git @Args
  if ($LASTEXITCODE -ne 0) {
    throw "git $($Args -join ' ') a echoue (code $LASTEXITCODE)"
  }
  if ($null -eq $output) {
    return ""
  }
  return (($output | Out-String).Trim())
}

function Convert-MetadataLine {
  param([string]$Line)
  if ([string]::IsNullOrWhiteSpace($Line)) {
    return @{}
  }
  $parts = $Line -split "\|", 4
  return @{
    commit = if ($parts.Length -gt 0) { $parts[0] } else { "" }
    short = if ($parts.Length -gt 1) { $parts[1] } else { "" }
    date = if ($parts.Length -gt 2) { $parts[2] } else { "" }
    subject = if ($parts.Length -gt 3) { $parts[3] } else { "" }
  }
}

try {
  Write-Host "=== Mise a jour Nautile depuis Git ===" -ForegroundColor Cyan

  if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw "git introuvable : impossible de mettre Nautile a jour automatiquement."
  }

  Invoke-GitChecked -C $RepoRoot rev-parse --is-inside-work-tree | Out-Null

  $branch = Invoke-GitText -C $RepoRoot rev-parse --abbrev-ref HEAD

  if ($branch -eq "HEAD") {
    throw "Depot en HEAD detache : impossible de savoir quelle branche mettre a jour."
  }

  $upstream = (& git -C $RepoRoot rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>$null)
  if ($LASTEXITCODE -ne 0 -or $null -eq $upstream) {
    $upstream = ""
  } else {
    $upstream = (($upstream | Out-String).Trim())
  }
  if ([string]::IsNullOrWhiteSpace($upstream)) {
    throw "La branche '$branch' n'a pas d'upstream Git. Configure-la avec : git branch --set-upstream-to=origin/$branch $branch"
  }

  Write-Host "Branche : $branch -> $upstream" -ForegroundColor DarkCyan
  Invoke-GitChecked -C $RepoRoot fetch --prune
  Invoke-GitChecked -C $RepoRoot pull --ff-only

  $targetDir = Join-Path $RepoRoot "target"
  $nautileCache = Join-Path $targetDir "nautile-navigateur.git"
  $nautileEnv = Join-Path $targetDir "nautile-upstream-version.env"
  if (-not (Test-Path $targetDir)) {
    New-Item -ItemType Directory -Path $targetDir | Out-Null
  }
  if (-not (Test-Path $nautileCache)) {
    Invoke-GitChecked clone --bare --filter=blob:none $NautileRepoUrl $nautileCache
  } else {
    Invoke-GitChecked -C $nautileCache fetch --prune origin "+refs/heads/*:refs/heads/*"
  }
  $nautileRef = "refs/heads/$NautileBranch"
  $upstreamMergeLine = Invoke-GitText -C $nautileCache log --merges -1 --date=short --format="%H|%h|%cd|%s" $nautileRef
  if ([string]::IsNullOrWhiteSpace($upstreamMergeLine)) {
    $upstreamMergeLine = Invoke-GitText -C $nautileCache log -1 --date=short --format="%H|%h|%cd|%s" $nautileRef
  }
  $upstreamMeta = Convert-MetadataLine $upstreamMergeLine
  @(
    "repo=$NautileRepoUrl"
    "branch=$NautileBranch"
    "commit=$($upstreamMeta.commit)"
    "short=$($upstreamMeta.short)"
    "date=$($upstreamMeta.date)"
    "subject=$($upstreamMeta.subject)"
  ) | Set-Content -Encoding ASCII $nautileEnv

  $nautileMerge = Invoke-GitText -C $RepoRoot log --merges -1 --date=short --format="%h %cd %s" -- src/browser
  if ([string]::IsNullOrWhiteSpace($nautileMerge)) {
    $nautileMerge = Invoke-GitText -C $RepoRoot log -1 --date=short --format="%h %cd %s" -- src/browser
  }
  $nautileSource = Invoke-GitText -C $RepoRoot log -1 --date=short --format="%h %cd %s" -- src/browser

  Write-Host "Nautile/Bouchaud OS est a jour avant bootimage." -ForegroundColor Green
  Write-Host "Nautile upstream ($NautileBranch) : $($upstreamMeta.short) $($upstreamMeta.date) $($upstreamMeta.subject)" -ForegroundColor Green
  Write-Host "Nautile dernier merge compile : $nautileMerge" -ForegroundColor Green
  Write-Host "Nautile dernier changement source : $nautileSource" -ForegroundColor DarkGreen
  Write-Host "Ces references seront injectees dans la banniere de boot par build.rs." -ForegroundColor DarkCyan
  exit 0
} catch {
  Write-Host $_.Exception.Message -ForegroundColor Red
  exit 1
}
