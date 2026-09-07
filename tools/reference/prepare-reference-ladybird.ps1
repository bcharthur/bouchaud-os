param(
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

$Image = Join-Path $RepoRoot "ladybird-browser.img"
$Verify = Join-Path $RepoRoot "tools\reference\verify-reference-ladybird-image.py"
$Make = Join-Path $RepoRoot "tools\reference\make-reference-ladybird-image.py"
$ArtifactVerify = Join-Path $RepoRoot "tools\reference\verify-ladybird-ramonly-artifact.py"
$Native = Join-Path $RepoRoot "native-browser-m9"
$Scenario = Join-Path $RepoRoot "scenario-stage2-ladybird"

Write-Host "=== Stage 2 FINAL V2 - preparation Ladybird ===" -ForegroundColor Cyan

if ((Test-Path -LiteralPath $Image -PathType Leaf) -and -not $Force) {
    & python $Verify $Image
    if ($LASTEXITCODE -eq 0) {
        Write-Host "Ladybird : image existante valide, reutilisee." -ForegroundColor Green
        exit 0
    }
    Write-Host "Ladybird : image existante invalide, reconstruction." -ForegroundColor Yellow
}

$Required = @(
    "BouchaudBrowserHost",
    "WebContent",
    "RequestServer",
    "ImageDecoder",
    "Compositor",
    "WebWorker",
    "WebDriver",
    "webcontent-bootstrap",
    "M9_CAPABLE",
    "V16_UI_CAPABLE",
    "V19_UI_CAPABLE"
)

if (-not (Test-Path -LiteralPath $Native -PathType Container)) {
    Fail (
        "native-browser-m9 absent. Le plus simple est de lancer le pipeline Ladybird " +
        "habituel une fois (.\run.ps1 -Ladybird), puis relancer ce script."
    )
}

foreach ($Name in $Required) {
    $Path = Join-Path $Native $Name
    if (-not (Test-Path -LiteralPath $Path)) {
        Fail "artefact Ladybird incomplet : $Name absent dans native-browser-m9"
    }
}

& python $ArtifactVerify (Join-Path $Native "BouchaudBrowserHost")
if ($LASTEXITCODE -ne 0) {
    Fail (
        "native-browser-m9 est une ancienne generation BrowserHost (persist/sql/cache disque). " +
        "Lance .\tools\reference\refresh-ladybird-ramonly-ci.ps1"
    )
}

$Resources = Join-Path $Native "resources"
if (-not (Test-Path -LiteralPath $Resources -PathType Container)) {
    Fail "artefact Ladybird incomplet : resources absent"
}

if (Test-Path -LiteralPath $Scenario) {
    Remove-Item -LiteralPath $Scenario -Recurse -Force
}

$Libexec = Join-Path $Scenario "usr\libexec\ladybird"
$Share = Join-Path $Scenario "usr\share\ladybird"
New-Item -ItemType Directory -Path $Libexec -Force | Out-Null
New-Item -ItemType Directory -Path $Share -Force | Out-Null

foreach ($Service in @(
    "BouchaudBrowserHost",
    "WebContent",
    "RequestServer",
    "ImageDecoder",
    "Compositor",
    "WebWorker",
    "WebDriver",
    "webcontent-bootstrap"
)) {
    Copy-Item `
        -LiteralPath (Join-Path $Native $Service) `
        -Destination (Join-Path $Libexec $Service) `
        -Force
}

Copy-Item `
    -LiteralPath (Join-Path $Native "BouchaudBrowserHost") `
    -Destination (Join-Path $Scenario "bo-navigateur") `
    -Force

Copy-Item `
    -Path (Join-Path $Resources "*") `
    -Destination $Share `
    -Recurse `
    -Force

$Fontconfig = Join-Path $RepoRoot "tools\ladybird\fontconfig\fonts.conf"
if (-not (Test-Path -LiteralPath $Fontconfig -PathType Leaf)) {
    Fail "fonts.conf Ladybird absent: $Fontconfig"
}
$FontconfigTarget = Join-Path $Share "fontconfig"
New-Item -ItemType Directory -Path $FontconfigTarget -Force | Out-Null
Copy-Item `
    -LiteralPath $Fontconfig `
    -Destination (Join-Path $FontconfigTarget "fonts.conf") `
    -Force

$CA = Join-Path $RepoRoot "tools\ladybird\certs\cacert.pem"
if (-not (Test-Path -LiteralPath $CA -PathType Leaf)) {
    Fail "bundle CA Ladybird absent: $CA"
}
$CertTarget = Join-Path $Scenario "etc\ssl\certs"
New-Item -ItemType Directory -Path $CertTarget -Force | Out-Null
Copy-Item `
    -LiteralPath $CA `
    -Destination (Join-Path $CertTarget "ca-certificates.crt") `
    -Force

$MarkerDir = Join-Path $Scenario "etc"
New-Item -ItemType Directory -Path $MarkerDir -Force | Out-Null
[System.IO.File]::WriteAllText(
    (Join-Path $MarkerDir "bouchaud-stage2-ladybird"),
    "Stage 2 FINAL V2 Ladybird`n",
    [System.Text.UTF8Encoding]::new($false)
)

& python $Make $Scenario $Image
if ($LASTEXITCODE -ne 0) {
    Fail "fabrication ladybird-browser.img en echec"
}

& python $Verify $Image
if ($LASTEXITCODE -ne 0) {
    Fail "validation ladybird-browser.img en echec"
}

Write-Host ""
Write-Host "BOUCHAUD_STAGE2_LADYBIRD_PREPARED" -ForegroundColor Green
