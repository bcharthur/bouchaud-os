param(
    [Alias("Url")]
    [string]$Open = "",

    [string]$Search = "",

    [ValidateSet("google", "duckduckgo")]
    [string]$SearchEngine = "google",

    [ValidateRange(2048, 16384)]
    [int]$RamMiB = 8192,

    [ValidateRange(1, 16)]
    [int]$CpuCount = 1,

    [switch]$M8,
    [switch]$M9Test,
    [switch]$Fullscreen,
    [switch]$RefreshLadybird,
    [switch]$RefreshCABundle,
    [switch]$AllowStaleArtifact,

    [long]$LadybirdRunId = 0,

    [string]$Audio = "dsound",
    [string]$Accel = "auto",
    [switch]$Sync
)

$ErrorActionPreference = "Stop"

$RepoRoot = $PSScriptRoot
$RunScript = Join-Path $RepoRoot "run.ps1"

if (-not (Test-Path -LiteralPath $RunScript -PathType Leaf)) {
    throw "run.ps1 introuvable : $RunScript"
}

if ($M8 -and $M9Test) {
    throw "Utiliser soit -M8, soit -M9Test."
}

if (-not [string]::IsNullOrWhiteSpace($Open) -and
    -not [string]::IsNullOrWhiteSpace($Search)) {
    throw "Utiliser soit -Open, soit -Search."
}

function Convert-ToSearchUrl {
    param([string]$Text, [string]$Engine)

    $encoded = [uri]::EscapeDataString($Text.Trim())

    switch ($Engine) {
        "google"     { return "https://www.google.com/search?q=$encoded" }
        "duckduckgo" { return "https://duckduckgo.com/?q=$encoded" }
        default      { throw "Moteur inconnu : $Engine" }
    }
}

function Resolve-NavigationInput {
    param([string]$Value)

    $candidate = $Value.Trim()

    if ($candidate -match '^[a-zA-Z][a-zA-Z0-9+.-]*://') {
        return $candidate
    }

    if ($candidate -match '^[^\s/]+\.[^\s]+') {
        return "https://$candidate"
    }

    return Convert-ToSearchUrl -Text $candidate -Engine $SearchEngine
}

function Ensure-PublicCABundle {
    $certDir = Join-Path $RepoRoot "tools\ladybird\certs"
    $bundle = Join-Path $certDir "cacert.pem"
    $provenance = Join-Path $certDir "README-local.txt"

    if ((Test-Path -LiteralPath $bundle -PathType Leaf) -and -not $RefreshCABundle) {
        $count = (Select-String -LiteralPath $bundle -Pattern "BEGIN CERTIFICATE" -AllMatches).Count
        if ($count -ge 100) {
            Write-Host "CA HTTPS   : bundle local present ($count certificats)" -ForegroundColor DarkGray
            return $bundle
        }
    }

    New-Item -ItemType Directory -Force -Path $certDir | Out-Null

    $pemUrl = "https://curl.se/ca/cacert.pem"
    $shaUrl = "https://curl.se/ca/cacert.pem.sha256"
    $tmpPem = Join-Path $env:TEMP "bouchaud-cacert.pem"
    $tmpSha = Join-Path $env:TEMP "bouchaud-cacert.pem.sha256"

    Remove-Item $tmpPem, $tmpSha -Force -ErrorAction SilentlyContinue

    Write-Host "CA HTTPS   : telechargement du bundle Mozilla via curl.se..." -ForegroundColor Yellow
    Invoke-WebRequest -UseBasicParsing -Uri $pemUrl -OutFile $tmpPem
    Invoke-WebRequest -UseBasicParsing -Uri $shaUrl -OutFile $tmpSha

    $shaText = (Get-Content -LiteralPath $tmpSha -Raw).Trim()
    if ($shaText -notmatch '(?i)\b([0-9a-f]{64})\b') {
        throw "Empreinte SHA-256 officielle illisible depuis curl.se."
    }

    $expected = $Matches[1].ToLowerInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $tmpPem).Hash.ToLowerInvariant()

    if ($actual -ne $expected) {
        throw "Bundle CA refuse : SHA-256 attendu $expected, obtenu $actual."
    }

    $count = (Select-String -LiteralPath $tmpPem -Pattern "BEGIN CERTIFICATE" -AllMatches).Count
    if ($count -lt 100) {
        throw "Bundle CA refuse : seulement $count certificats."
    }

    Move-Item -LiteralPath $tmpPem -Destination $bundle -Force
    Remove-Item $tmpSha -Force -ErrorAction SilentlyContinue

    @"
Bundle local pour Bouchaud OS.
Source : curl CA Extract / Mozilla trust store
URL source : https://curl.se/docs/caextract.html
SHA-256 verifie : $actual
Certificats : $count
Licence du bundle : MPL 2.0 (heritage Mozilla)
Genere : $(Get-Date -Format o)
"@ | Set-Content -LiteralPath $provenance -Encoding UTF8

    Write-Host "CA HTTPS   : $count certificats, SHA-256 verifie" -ForegroundColor Green
    return $bundle
}

function Find-CurrentHeadLadybirdRun {
    if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
        throw "GitHub CLI 'gh' est requis pour verifier l'artefact Ladybird courant."
    }

    $head = (& git -C $RepoRoot rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $head) {
        throw "Impossible de determiner HEAD."
    }

    $json = & gh run list `
        --repo "bcharthur/bouchaud-os" `
        --workflow "ladybird-native-browser.yml" `
        --status success `
        --limit 50 `
        --json databaseId,headSha,headBranch,createdAt

    if ($LASTEXITCODE -ne 0) {
        throw "Impossible de lister les runs GitHub Actions."
    }

    $runs = @($json | ConvertFrom-Json)
    $match = $runs |
        Where-Object { $_.headSha -eq $head } |
        Sort-Object createdAt -Descending |
        Select-Object -First 1

    if (-not $match) {
        throw @"
Aucun artefact Ladybird REUSSI ne correspond au HEAD local :
  $head

La regression HTTP M9 locale peut continuer avec :
  .\run-ladybird-local.ps1

Pour Internet/Google, je refuse d'utiliser silencieusement l'ancien artefact
M9 : il ne contient pas forcement DNS/HTTPS de ce HEAD.

Il faut d'abord obtenir un workflow ladybird-native-browser vert pour ce HEAD.
Pour un essai volontaire avec un artefact ancien :
  -AllowStaleArtifact
"@
    }

    Write-Host "artefact   : run $($match.databaseId), HEAD exact $head" -ForegroundColor Green
    return [long]$match.databaseId
}

$RemoteNavigation = $false

if (-not [string]::IsNullOrWhiteSpace($Search)) {
    $EffectiveUrl = Convert-ToSearchUrl -Text $Search -Engine $SearchEngine
    $RemoteNavigation = $true
}
elseif (-not [string]::IsNullOrWhiteSpace($Open)) {
    $EffectiveUrl = Resolve-NavigationInput -Value $Open
    $RemoteNavigation = $EffectiveUrl -match '^https?://' -and
        -not $EffectiveUrl.StartsWith("http://10.0.2.2:18080/")
}
else {
    $EffectiveUrl = "http://10.0.2.2:18080/m9.html"
}

if ($RemoteNavigation -and $EffectiveUrl -match '^https://') {
    [void](Ensure-PublicCABundle)
}

$EffectiveRunId = $LadybirdRunId
$ForceRefreshArtifact = [bool]$RefreshLadybird

if ($RemoteNavigation -and -not $AllowStaleArtifact) {
    if ($EffectiveRunId -eq 0) {
        $EffectiveRunId = Find-CurrentHeadLadybirdRun
    }
    $ForceRefreshArtifact = $true
}
elseif ($RemoteNavigation -and $AllowStaleArtifact) {
    Write-Host "ATTENTION   : artefact stale autorise ; DNS/HTTPS du HEAD peuvent manquer." -ForegroundColor Yellow
}

$RunParams = @{
    LadybirdUrl      = $EffectiveUrl
    LadybirdRamMiB   = $RamMiB
    LadybirdCpuCount = $CpuCount
    Audio            = $Audio
    Accel            = $Accel
}

if ($M8) {
    $RunParams["LadybirdM8"] = $true
}
elseif ($M9Test) {
    $RunParams["LadybirdM9Test"] = $true
}
else {
    $RunParams["Ladybird"] = $true
}

if ($Fullscreen) { $RunParams["Fullscreen"] = $true }
if ($ForceRefreshArtifact) { $RunParams["RefreshLadybird"] = $true }
if ($EffectiveRunId -ne 0) { $RunParams["LadybirdRunId"] = $EffectiveRunId }
if ($Sync) { $RunParams["Sync"] = $true }

Write-Host ""
Write-Host "=== Bouchaud Ladybird HW/Web v3 ===" -ForegroundColor Cyan
Write-Host "URL        : $EffectiveUrl" -ForegroundColor Green
Write-Host "RAM        : $RamMiB Mio" -ForegroundColor DarkGray
Write-Host "vCPU       : $CpuCount (1 utile tant que SMP guest n'est pas porte)" -ForegroundColor DarkGray
Write-Host ""

& $RunScript @RunParams
exit $LASTEXITCODE
