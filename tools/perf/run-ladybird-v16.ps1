param(
    [string]$Url = "https://www.google.com/",
    [ValidateRange(4096,16384)][int]$RamMiB = 12288,
    [ValidateSet(1,4,8)][int]$CpuCount = 4,
    [ValidateSet("tcg","whpx","auto")][string]$Accel = "tcg",
    [switch]$Audio,
    [switch]$RefreshLadybird
)
$ErrorActionPreference="Stop"
$RepoRoot=(Resolve-Path (Join-Path $PSScriptRoot "..\..")).Path
$Run=Join-Path $RepoRoot "run.ps1"
$Native=Join-Path $RepoRoot "native-browser-m9"
$Marker=Join-Path $Native "V16_UI_CAPABLE"

function Get-V16Artifact {
    Write-Host "V16: artefact navigateur local ancien/absent -> recherche CI" -ForegroundColor Yellow
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { throw "GitHub CLI 'gh' requis pour recuperer Ladybird V16." }
    $branch=(& git -C $RepoRoot branch --show-current).Trim()
    if (-not $branch) { throw "Branche Git introuvable." }

    # V16.1 : ne filtrer PAS sur --status success ici. Au premier build V16,
    # il n'existe par definition aucun run reussi pendant que la CI compile.
    # On prend le run V16 le plus recent, on l'attend s'il est encore actif,
    # puis on telecharge exactement son artefact s'il termine en succes.
    $raw = (& gh run list `
        --workflow "ladybird-native-browser-v16.yml" `
        --branch $branch `
        --limit 1 `
        --json databaseId,status,conclusion,url) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw "Impossible d'interroger GitHub Actions pour Ladybird V16." }
    $runs = @($raw | ConvertFrom-Json)
    if ($runs.Count -eq 0) {
        throw "Aucun workflow Ladybird V16 trouve pour '$branch'. Pousse le patch V16 d'abord."
    }

    $latest = $runs[0]
    $runId = [long]$latest.databaseId
    Write-Host ("V16 CI: run {0}, status={1}, conclusion={2}" -f $runId, $latest.status, $latest.conclusion) -ForegroundColor DarkGray

    if ($latest.status -ne "completed") {
        Write-Host "V16 CI encore en cours : attente de la fin du workflow..." -ForegroundColor Yellow
        & gh run watch $runId --exit-status
        if ($LASTEXITCODE -ne 0) {
            throw "Le workflow Ladybird V16 $runId a echoue. Consulte: $($latest.url)"
        }
    }

    $viewRaw = (& gh run view $runId --json status,conclusion,url) -join "`n"
    if ($LASTEXITCODE -ne 0) { throw "Impossible de relire l'etat du workflow V16 $runId." }
    $view = $viewRaw | ConvertFrom-Json
    if ($view.status -ne "completed" -or $view.conclusion -ne "success") {
        throw "Le workflow Ladybird V16 $runId n'est pas exploitable: status=$($view.status) conclusion=$($view.conclusion). Consulte: $($view.url)"
    }

    $tmp=Join-Path $RepoRoot ".v16-ladybird-download"
    if (Test-Path $tmp) { Remove-Item -Recurse -Force $tmp }
    New-Item -ItemType Directory -Path $tmp | Out-Null
    & gh run download $runId -n "bouchaud-ladybird-native-browser" -D $tmp
    if ($LASTEXITCODE -ne 0) { throw "Telechargement artefact V16 echoue pour le run $runId." }
    if (-not (Test-Path (Join-Path $tmp "V16_UI_CAPABLE"))) {
        Remove-Item -Recurse -Force $tmp
        throw "L'artefact du run $runId n'est pas V16_UI_CAPABLE. Le chrome bitmap est refuse."
    }
    foreach($f in @("BouchaudBrowserHost","WebContent","RequestServer","ImageDecoder","Compositor","V16_UI_CAPABLE")) {
        if (-not (Test-Path (Join-Path $tmp $f))) { throw "Artefact V16 incomplet: $f" }
    }
    if (Test-Path $Native) { Remove-Item -Recurse -Force $Native }
    Move-Item $tmp $Native
    Write-Host "V16: artefact UI verifie installe ($runId)" -ForegroundColor Green
}

if ($RefreshLadybird -or -not (Test-Path $Marker)) { Get-V16Artifact }
Write-Host "V16 Typography / Fluidity" -ForegroundColor Cyan
Write-Host "vCPU=$CpuCount accel=$Accel RAM=${RamMiB}MiB artifact=V16" -ForegroundColor DarkGray
if ($CpuCount -eq 8 -and $Accel -eq "tcg") { Write-Host "TCG8 peut augmenter la contention; comparer d'abord SMP4." -ForegroundColor Yellow }
if ($Accel -eq "whpx" -and $CpuCount -gt 1) { Write-Host "WHPX SMP reste experimental sur le bring-up APIC actuel." -ForegroundColor Yellow }
$RunParams=@{ Ladybird=$true; LadybirdUrl=$Url; RamMiB=$RamMiB; CpuCount=$CpuCount; Accel=$Accel; Audio=($(if($Audio){"dsound"}else{"none"})) }
& $Run @RunParams
exit $LASTEXITCODE
