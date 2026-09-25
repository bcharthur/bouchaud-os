[CmdletBinding()]
param(
    [string]$RepoRoot = "",
    [switch]$Apply,
    [switch]$Deep
)
$ErrorActionPreference = "Stop"

if (-not $RepoRoot) {
    $ToolsDir = Split-Path -Parent $PSScriptRoot
    $RepoRoot = Split-Path -Parent $ToolsDir
}
$RepoRoot = [System.IO.Path]::GetFullPath($RepoRoot)

if (-not (Test-Path (Join-Path $RepoRoot ".git"))) {
    throw "Racine Git invalide: $RepoRoot"
}

function Relative-Path([string]$Path) {
    # Compatible Windows PowerShell 5.1 : Path.GetRelativePath n'existe pas
    # sur le .NET Framework livre avec PowerShell historique.
    $RootWithSlash = $RepoRoot.TrimEnd('\') + '\'
    $RootUri = New-Object System.Uri($RootWithSlash)
    $PathUri = New-Object System.Uri([System.IO.Path]::GetFullPath($Path))
    return [System.Uri]::UnescapeDataString($RootUri.MakeRelativeUri($PathUri).ToString()).Replace('/','\')
}

function Has-TrackedFiles([string]$Path) {
    $Rel = (Relative-Path $Path).Replace('\','/')
    $Tracked = & git -C $RepoRoot ls-files -- $Rel
    if ($LASTEXITCODE -ne 0) { throw "git ls-files a echoue pour $Rel" }
    return [bool]($Tracked | Select-Object -First 1)
}

function Add-Candidate([System.Collections.Generic.List[string]]$List, [string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $Full = [System.IO.Path]::GetFullPath($Path)
    if ($Full -eq $RepoRoot -or $Full.StartsWith((Join-Path $RepoRoot ".git"))) { return }
    if (Has-TrackedFiles $Full) {
        Write-Host "SKIP suivi par Git: $(Relative-Path $Full)" -ForegroundColor Yellow
        return
    }
    if (-not $List.Contains($Full)) { $List.Add($Full) }
}

$Candidates = [System.Collections.Generic.List[string]]::new()

# Scaffolds de hotfix / validation crees a la racine. Ils sont regenerables et
# ne sont jamais des sources. Le test Has-TrackedFiles reste l'autorite finale.
$RootPatterns = @(
    ".patch-backup",
    ".hotfix*-last-backup",
    "HOTFIX*",
    "STRESS-*",
    "P0-REMOTE-CONTROL-V*",
    "BOUCHAUD-LIVE-AND-STRESS-V*",
    "ANALYSE-LADYBIRD-*.ps1",
    "APPLY-LADYBIRD-*.ps1",
    "FINALIZE-LADYBIRD-*.ps1"
)
foreach ($Pattern in $RootPatterns) {
    Get-ChildItem -LiteralPath $RepoRoot -Force -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -like $Pattern } |
        ForEach-Object { Add-Candidate $Candidates $_.FullName }
}

# Bytecode Python, y compris sous des repertoires deja ignores.
foreach ($ScanRoot in @((Join-Path $RepoRoot "tools"), (Join-Path $RepoRoot "src"))) {
    if (Test-Path $ScanRoot) {
        Get-ChildItem -LiteralPath $ScanRoot -Directory -Recurse -Force -Filter "__pycache__" -ErrorAction SilentlyContinue |
            ForEach-Object { Add-Candidate $Candidates $_.FullName }
    }
}

if ($Deep) {
    # Gros arbres regenerables. Ils ne sont PAS supprimes dans le nettoyage
    # normal, car les reconstruire coute du temps.
    foreach ($Pattern in @(
        "native-browser-m9",
        "native-browser-m9.previous-*",
        "scenario-stage2-ladybird",
        ".ladybird-local-export*"
    )) {
        Get-ChildItem -LiteralPath $RepoRoot -Force -ErrorAction SilentlyContinue |
            Where-Object { $_.Name -like $Pattern } |
            ForEach-Object { Add-Candidate $Candidates $_.FullName }
    }

    $Target = Join-Path $RepoRoot "target"
    if (Test-Path $Target) {
        foreach ($Pattern in @(
            "blackbox-extract-*",
            "hotfix*-*",
            "live-observability-*",
            "physical-stress-*",
            "remote-dump-*",
            "stress-v4-*"
        )) {
            Get-ChildItem -LiteralPath $Target -Force -ErrorAction SilentlyContinue |
                Where-Object { $_.Name -like $Pattern } |
                ForEach-Object { Add-Candidate $Candidates $_.FullName }
        }
    }
}

$Candidates = @($Candidates | Sort-Object -Unique)
Write-Host "=== NETTOYAGE LOCAL BOUCHAUD OS ===" -ForegroundColor Cyan
Write-Host "Repo : $RepoRoot"
Write-Host "Mode : $(if ($Apply) {'APPLY'} else {'DRY-RUN'})$(if ($Deep) {' + DEEP'} else {''})"
Write-Host "Candidats : $($Candidates.Count)"

foreach ($Path in $Candidates) {
    $Rel = Relative-Path $Path
    if ($Apply) {
        Write-Host "REMOVE $Rel" -ForegroundColor DarkYellow
        Remove-Item -LiteralPath $Path -Recurse -Force
    } else {
        Write-Host "WOULD REMOVE $Rel"
    }
}

Write-Host ""
if (-not $Apply) {
    Write-Host "Aucune suppression effectuee. Relancer avec -Apply apres lecture de la liste." -ForegroundColor Green
} else {
    Write-Host "Nettoyage termine. Aucun fichier suivi par Git n'a ete volontairement supprime." -ForegroundColor Green
}

Write-Host ""
Write-Host "--- git status --short ---"
& git -C $RepoRoot status --short
