# Lancement d'un run Ladybird sur le SHA distant exact, sans modification Git.
# Usage : .\tools\ci\run-bouchaud-ladybird-ci.ps1 [-Watch]
[CmdletBinding()]
param(
    [string]$Repo = (Get-Location).Path,
    [string]$Branch = 'claude/ladybird-observability-performance',
    [string]$GitHubRepo = 'bcharthur/bouchaud-os',
    [switch]$Watch
)
$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $Repo
foreach ($cmd in @('git', 'gh')) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) { throw "$cmd introuvable" }
}
$localBranch = (git branch --show-current).Trim()
if ($LASTEXITCODE -ne 0 -or $localBranch -ne $Branch) { throw "Branche incorrecte : $localBranch" }
$status = @(git status --porcelain)
if ($LASTEXITCODE -ne 0 -or $status.Count -ne 0) { throw 'Checkout non propre : ne pas déclencher une preuve différente des sources locales' }
$sha = (git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $sha.Length -ne 40) { throw 'Git HEAD illisible' }
$remoteLine = @(git ls-remote origin "refs/heads/$Branch")
if ($LASTEXITCODE -ne 0 -or $remoteLine.Count -ne 1) { throw 'HEAD distant indéterminé' }
$remoteSha = (($remoteLine[0] -split '\s+')[0]).Trim()
if ($remoteSha -ne $sha) { throw "Local $sha != distant $remoteSha : synchroniser avec --ff-only avant le run" }
$wf = 'ladybird-native-browser.yml'
$beforeRaw = gh run list --repo $GitHubRepo --workflow $wf --branch $Branch --event workflow_dispatch --limit 30 --json databaseId,headSha
if ($LASTEXITCODE -ne 0) { throw 'Liste des runs inaccessible' }
$before = @($beforeRaw | ConvertFrom-Json)
$beforeIds = @($before | ForEach-Object { $_.databaseId })
Write-Host "LADYBIRD_SHA_VERIFIE $sha" -ForegroundColor Green
gh workflow run $wf --repo $GitHubRepo --ref $Branch
if ($LASTEXITCODE -ne 0) { throw 'Échec du workflow dispatch' }
$run = $null
for ($try = 0; $try -lt 18; $try++) {
    Start-Sleep -Seconds 5
    $raw = gh run list --repo $GitHubRepo --workflow $wf --branch $Branch --event workflow_dispatch --limit 30 --json databaseId,headSha,status
    if ($LASTEXITCODE -ne 0) { throw 'Recherche du run échouée' }
    $matches = @(($raw | ConvertFrom-Json) | Where-Object {
        $_.headSha -eq $sha -and $_.databaseId -notin $beforeIds
    })
    if ($matches.Count -eq 1) { $run = $matches[0]; break }
    if ($matches.Count -gt 1) { throw 'Plusieurs runs nouveaux : identité ambiguë ; inspecter GitHub' }
}
if (-not $run) { throw 'Run non retrouvé avec certitude : chercher manuellement dans Actions' }
Write-Host "LADYBIRD_RUN_ID=$($run.databaseId) SHA=$sha"
Write-Host "https://github.com/$GitHubRepo/actions/runs/$($run.databaseId)"
if (-not $Watch) { return }
gh run watch $run.databaseId --repo $GitHubRepo --exit-status
if ($LASTEXITCODE -ne 0) { throw 'Campagne Ladybird échouée : ne pas qualifier' }
$detailsRaw = gh run view $run.databaseId --repo $GitHubRepo --json headSha,conclusion,jobs
if ($LASTEXITCODE -ne 0) { throw 'Verdict final GitHub inaccessible' }
$details = $detailsRaw | ConvertFrom-Json
if ($details.headSha -ne $sha -or $details.conclusion -ne 'success') { throw 'SHA ou conclusion finale invalide' }
$convergence = @($details.jobs | Where-Object { $_.name -match 'verdict de convergence' })
if ($convergence.Count -ne 1 -or $convergence[0].conclusion -ne 'success') {
    throw 'Convergence non prouvée dans les jobs'
}
Write-Host "LADYBIRD_CONVERGENCE_JOB_SUCCESS_SHA=$sha" -ForegroundColor Green
