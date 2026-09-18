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
        "native-browser-m9 absent. Trois voies, de la plus rapide a la plus " +
        "longue :`n" +
        "  1. .\tools\reference\recupere-navigateur-ci.ps1`n" +
        "     Recupere l'artefact que la CI a deja construit ET verifie " +
        "(environ 430 Mio, quelques minutes). C'est la voie normale.`n" +
        "  2. Si un AUTRE arbre de travail possede deja " +
        "ladybird-browser.img -- c'est le cas d'un worktree cree a cote de " +
        "l'arbre principal --, reprends-la avec " +
        "IMAGE-TRIGKEY.ps1 -LadybirdDepuis <cet-autre-arbre>.`n" +
        "  3. .\run.ps1 -Ladybird : reconstruire Ladybird de bout en bout. " +
        "vcpkg (Skia, ICU, HarfBuzz) puis LibWeb et les services : des heures " +
        "et plusieurs dizaines de gibioctets. A ne faire que si les deux " +
        "premieres voies sont fermees."
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

# BOUCHAUD_PAGE_ACCUEIL_DEPUIS_LE_DEPOT_V1
#
# La page d'accueil est a NOUS, pas au navigateur. tools/ladybird/start.html
# n'etait installee que par tools/ladybird/browser-upstream.sh, donc seulement
# par la reconstruction integrale de Ladybird (des heures). L'artefact publie
# par la CI est construit sur `main`, ou start.html n'a jamais ete poussee :
# son resources/ ne pouvait pas la contenir, et Stage 2 ouvrait quand meme
# file:///usr/share/ladybird/bouchaud-start.html. Resultat a l'ecran :
# "Load failed: No such file or directory (errno=2)", 9,4 s apres le
# lancement -- un navigateur parfaitement fonctionnel, sur une page absente.
#
# fonts.conf et le bundle CA, juste en dessous, ont deja ce traitement pour
# exactement la meme raison. La page d'accueil le rejoint : elle est copiee
# APRES resources/, donc le depot fait autorite sur notre propre fichier quel
# que soit l'age de l'artefact.
$PageAccueil = Join-Path $RepoRoot "tools\ladybird\start.html"
if (-not (Test-Path -LiteralPath $PageAccueil -PathType Leaf)) {
    Fail "page d'accueil Ladybird absente: $PageAccueil"
}
Copy-Item `
    -LiteralPath $PageAccueil `
    -Destination (Join-Path $Share "bouchaud-start.html") `
    -Force

# BOUCHAUD_CA_BUNDLE_AUTO_V1
# Le bundle CA est volontairement local et ignore par Git. S'il manque,
# le fabriquer depuis le magasin de certificats racine Windows (ou, a defaut,
# depuis les racines DER versionnees du noyau) avant de construire l'image.
$CA = Join-Path $RepoRoot "tools\ladybird\certs\cacert.pem"
$CABuilder = Join-Path $RepoRoot "tools\ladybird\certs\fabrique-bundle.ps1"

if (-not (Test-Path -LiteralPath $CA -PathType Leaf)) {
    if (-not (Test-Path -LiteralPath $CABuilder -PathType Leaf)) {
        Fail "generateur de bundle CA Ladybird absent: $CABuilder"
    }

    Write-Host "Ladybird : bundle CA absent, generation locale..." -ForegroundColor Yellow
    & powershell -NoProfile -ExecutionPolicy Bypass -File $CABuilder
    if ($LASTEXITCODE -ne 0) {
        Fail "generation du bundle CA Ladybird en echec"
    }
}

if (-not (Test-Path -LiteralPath $CA -PathType Leaf)) {
    Fail "bundle CA Ladybird toujours absent apres generation: $CA"
}

$CaInfo = Get-Item -LiteralPath $CA
if ($CaInfo.Length -lt 1024) {
    Fail "bundle CA Ladybird anormalement petit: $($CaInfo.Length) octets"
}
Write-Host ("Ladybird : bundle CA pret ({0} octets)" -f $CaInfo.Length) -ForegroundColor Green

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

# La charge d'installation part dans la MEME archive : c'est elle que
# l'installateur du noyau lira pour ecrire l'ESP du disque interne.
& powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $RepoRoot "tools\reference\stage-install-payload.ps1") -Scenario $Scenario
if ($LASTEXITCODE -ne 0) { Fail "charge d'installation en echec" }

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
