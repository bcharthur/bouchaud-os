param(
    [long]$RunId = 0,
    [switch]$SkipUsbBuild
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot

function Fail([string]$Message) {
    Write-Host ""
    Write-Host "ERREUR: $Message" -ForegroundColor Red
    exit 1
}

function Require-Command([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        Fail "$Name introuvable"
    }
}

function Get-DispatchRuns([string]$Workflow, [string]$Branch) {
    $json = & gh run list `
        --workflow $Workflow `
        --branch $Branch `
        --event workflow_dispatch `
        --limit 30 `
        --json databaseId,headSha,status,conclusion,createdAt 2>$null
    if ($LASTEXITCODE -ne 0) {
        Fail "impossible de lister les runs GitHub Actions"
    }
    if (-not $json) {
        return @()
    }
    return @($json | ConvertFrom-Json)
}

Require-Command "git"
Require-Command "gh"
Require-Command "python"

& gh auth status
if ($LASTEXITCODE -ne 0) {
    Fail "GitHub CLI n'est pas authentifie"
}

$Branch = (& git branch --show-current).Trim()
$Head = (& git rev-parse HEAD).Trim()

if ($Branch -ne "feat/reference-trigkey-stage1-uefi") {
    Fail "branche inattendue: $Branch"
}
if (-not $Head.StartsWith("5da5939")) {
    Fail "HEAD inattendu: $Head. V2.4 est base sur le checkpoint 5da5939."
}

$Workflow = "ladybird-native-browser.yml"
$ArtifactName = "bouchaud-ladybird-native-browser"
$VerifyArtifact = Join-Path $RepoRoot "tools\reference\verify-ladybird-ramonly-artifact.py"
$Native = Join-Path $RepoRoot "native-browser-m9"
$LadybirdImage = Join-Path $RepoRoot "ladybird-browser.img"

Write-Host "=== Bouchaud Stage 2 V2.4 - Ladybird RAM-only ===" -ForegroundColor Cyan
Write-Host "Branche : $Branch"
Write-Host "HEAD    : $Head"
Write-Host ""

if ($RunId -eq 0) {
    $Before = Get-DispatchRuns -Workflow $Workflow -Branch $Branch
    $BeforeIds = @{}
    foreach ($run in $Before) {
        $BeforeIds[[string]$run.databaseId] = $true
    }

    Write-Host "Declenchement du workflow canonique Ladybird..." -ForegroundColor Yellow
    & gh workflow run $Workflow --ref $Branch
    if ($LASTEXITCODE -ne 0) {
        Fail "workflow_dispatch Ladybird en echec"
    }

    $Deadline = (Get-Date).AddMinutes(3)
    $Selected = $null
    while ((Get-Date) -lt $Deadline -and -not $Selected) {
        Start-Sleep -Seconds 3
        $Runs = Get-DispatchRuns -Workflow $Workflow -Branch $Branch
        $Selected = $Runs |
            Where-Object {
                $_.headSha -eq $Head -and
                -not $BeforeIds.ContainsKey([string]$_.databaseId)
            } |
            Sort-Object createdAt -Descending |
            Select-Object -First 1
    }

    if (-not $Selected) {
        Fail (
            "le nouveau workflow_dispatch n'a pas ete retrouve sous 3 minutes. " +
            "Verifie Actions puis relance avec -RunId <id>."
        )
    }
    $RunId = [long]$Selected.databaseId
}

Write-Host "LADYBIRD_RAMONLY_RUN_ID=$RunId" -ForegroundColor Green
Write-Host "Le build Ladybird peut etre long; gh run watch reste attache." -ForegroundColor Yellow

& gh run watch $RunId --exit-status
if ($LASTEXITCODE -ne 0) {
    Fail "le workflow Ladybird $RunId n'est pas vert"
}

$Temp = Join-Path $env:TEMP ("bouchaud-v24-ladybird-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $Temp -Force | Out-Null

$NativeBackup = $null
$ImageBackup = $null
$NativeInstalled = $false

try {
    Write-Host ""
    Write-Host "Telechargement de l'artefact $ArtifactName..." -ForegroundColor Cyan
    & gh run download $RunId -n $ArtifactName -D $Temp
    if ($LASTEXITCODE -ne 0) {
        throw "telechargement artefact Ladybird en echec"
    }

    $HostArtifact = Get-ChildItem -LiteralPath $Temp -Recurse -File -Filter "BouchaudBrowserHost" |
        Select-Object -First 1
    if (-not $HostArtifact) {
        throw "BouchaudBrowserHost absent de l'artefact telecharge"
    }
    $ArtifactRoot = $HostArtifact.Directory.FullName

    foreach ($Required in @(
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
    )) {
        if (-not (Test-Path -LiteralPath (Join-Path $ArtifactRoot $Required))) {
            throw "artefact CI incomplet: $Required absent"
        }
    }
    if (-not (Test-Path -LiteralPath (Join-Path $ArtifactRoot "resources") -PathType Container)) {
        throw "artefact CI incomplet: resources absent"
    }

    & python $VerifyArtifact (Join-Path $ArtifactRoot "BouchaudBrowserHost")
    if ($LASTEXITCODE -ne 0) {
        throw "le run CI a produit un BrowserHost encore anterieur au mode RAM-only"
    }

    if (Test-Path -LiteralPath $Native) {
        $NativeBackup = "$Native.stale-v24"
        if (Test-Path -LiteralPath $NativeBackup) {
            Remove-Item -LiteralPath $NativeBackup -Recurse -Force
        }
        Rename-Item -LiteralPath $Native -NewName ([IO.Path]::GetFileName($NativeBackup))
    }

    New-Item -ItemType Directory -Path $Native -Force | Out-Null
    Copy-Item -Path (Join-Path $ArtifactRoot "*") -Destination $Native -Recurse -Force
    $NativeInstalled = $true

    & python $VerifyArtifact (Join-Path $Native "BouchaudBrowserHost")
    if ($LASTEXITCODE -ne 0) {
        throw "validation du nouvel artefact local en echec"
    }

    if (Test-Path -LiteralPath $LadybirdImage -PathType Leaf) {
        $ImageBackup = "$LadybirdImage.stale-v24"
        if (Test-Path -LiteralPath $ImageBackup) {
            Remove-Item -LiteralPath $ImageBackup -Force
        }
        Move-Item -LiteralPath $LadybirdImage -Destination $ImageBackup
    }

    Write-Host ""
    Write-Host "Reconstruction du ramdisk Ladybird RAM-only..." -ForegroundColor Cyan
    & ".\tools\reference\prepare-reference-ladybird.ps1" -Force
    if ($LASTEXITCODE -ne 0) {
        throw "reconstruction de ladybird-browser.img en echec"
    }

    & python ".\tools\reference\verify-reference-ladybird-image.py" $LadybirdImage
    if ($LASTEXITCODE -ne 0) {
        throw "nouvelle image Ladybird invalide"
    }

    if (-not $SkipUsbBuild) {
        Write-Host ""
        Write-Host "Reconstruction de l'image single-USB TRIGKEY..." -ForegroundColor Cyan
        & ".\tools\reference\build-trigkey-stage2-usb.ps1"
        if ($LASTEXITCODE -ne 0) {
            throw "build de l'image single-USB TRIGKEY en echec"
        }
    }

    if ($NativeBackup -and (Test-Path -LiteralPath $NativeBackup)) {
        Remove-Item -LiteralPath $NativeBackup -Recurse -Force
        $NativeBackup = $null
    }
    if ($ImageBackup -and (Test-Path -LiteralPath $ImageBackup)) {
        Remove-Item -LiteralPath $ImageBackup -Force
        $ImageBackup = $null
    }

    Write-Host ""
    Write-Host "BOUCHAUD_STAGE2_V24_RAMONLY_ARTIFACT_READY" -ForegroundColor Green
    if (-not $SkipUsbBuild) {
        Write-Host "BOUCHAUD_STAGE2_V24_RAMONLY_USB_READY" -ForegroundColor Green
    }
}
catch {
    Write-Host ""
    Write-Host "ERREUR V2.4: $($_.Exception.Message)" -ForegroundColor Red
    Write-Host "Restauration de l'artefact/image precedents..." -ForegroundColor Yellow

    if ($NativeInstalled -and (Test-Path -LiteralPath $Native)) {
        Remove-Item -LiteralPath $Native -Recurse -Force
    }
    if ($NativeBackup -and (Test-Path -LiteralPath $NativeBackup)) {
        Rename-Item -LiteralPath $NativeBackup -NewName ([IO.Path]::GetFileName($Native))
    }

    if ($ImageBackup -and (Test-Path -LiteralPath $ImageBackup)) {
        if (Test-Path -LiteralPath $LadybirdImage) {
            Remove-Item -LiteralPath $LadybirdImage -Force
        }
        Move-Item -LiteralPath $ImageBackup -Destination $LadybirdImage
    }

    Write-Host "BOUCHAUD_STAGE2_V24_RUNTIME_ROLLBACK_OK" -ForegroundColor Yellow
    exit 1
}
finally {
    if (Test-Path -LiteralPath $Temp) {
        Remove-Item -LiteralPath $Temp -Recurse -Force
    }
}
