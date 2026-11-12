# BOUCHAUD_WINDOWS_V1 -- fonctions communes des scripts Windows de Bouchaud OS.
#
# Regle : ces scripts ORCHESTRENT les outils du depot ; ils ne les
# reimplementent pas. Tout ce qui construit une image s'execute sous Linux :
# dans WSL (Windows) ou directement (pwsh sous Linux, utilise par la CI).
# ASCII strict (tools/verifie-scripts-windows.py).

Set-StrictMode -Version 3

$script:RacineDepot = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path

function Get-RacineDepot { return $script:RacineDepot }

function Test-SousWindows {
    return ($PSVersionTable.PSEdition -eq 'Desktop') -or ($IsWindows -eq $true)
}

function Write-Etape([string]$Texte) { Write-Host "==> $Texte" -ForegroundColor Cyan }
function Write-Ok([string]$Texte) { Write-Host "OK  $Texte" -ForegroundColor Green }
function Stop-Bouchaud([string]$Texte) {
    Write-Host "REFUS $Texte" -ForegroundColor Red
    throw "REFUS $Texte"
}

function Assert-Commande([string]$Nom, [string]$Aide) {
    if (-not (Get-Command $Nom -ErrorAction SilentlyContinue)) {
        Stop-Bouchaud "$Nom introuvable : $Aide"
    }
}

# Le SHA exact de HEAD, et refus d'un arbre sale : une image doit dire de quel
# commit elle vient, sans fichier oublie.
function Get-ShaGit([switch]$AccepteArbreSale) {
    Assert-Commande 'git' 'installer Git for Windows'
    Push-Location (Get-RacineDepot)
    try {
        $sha = (& git rev-parse HEAD).Trim()
        if ($LASTEXITCODE -ne 0 -or $sha -notmatch '^[0-9a-f]{40}$') { Stop-Bouchaud 'git rev-parse HEAD a echoue' }
        $sale = @(& git status --porcelain --untracked-files=normal)
        if ($sale.Count -gt 0 -and -not $AccepteArbreSale) {
            Stop-Bouchaud "arbre de travail sale ($($sale.Count) fichier(s)) : committer d'abord"
        }
        return $sha
    } finally { Pop-Location }
}

# Chemin Windows -> chemin vu de WSL (/mnt/c/...). Sous Linux : inchange.
function ConvertTo-CheminLinux([string]$Chemin) {
    $plein = [System.IO.Path]::GetFullPath($Chemin)
    if (-not (Test-SousWindows)) { return $plein }
    $p = (& wsl.exe wslpath -a -u ($plein -replace '\\', '/')).Trim()
    if ($LASTEXITCODE -ne 0 -or -not $p) { Stop-Bouchaud "wslpath a echoue pour $plein" }
    return $p
}

# Execute une commande bash dans le depot : WSL sous Windows, bash sous Linux.
# Rend le code de sortie ; la sortie est recopiee dans le journal.
function Invoke-Linux([string]$Commande) {
    $depot = ConvertTo-CheminLinux (Get-RacineDepot)
    $complete = "cd '$depot' && $Commande"
    Write-Host "    [linux] $Commande"
    if (Test-SousWindows) {
        Assert-Commande 'wsl.exe' 'activer WSL 2 (wsl --install -d Ubuntu), voir tools/windows/README.md'
        & wsl.exe -e bash -lc $complete
    } else {
        & bash -lc $complete
    }
    return $LASTEXITCODE
}

function Start-JournalBouchaud([string]$Nom) {
    $dossier = Join-Path (Get-RacineDepot) 'target/logs'
    New-Item -ItemType Directory -Force -Path $dossier | Out-Null
    $f = Join-Path $dossier ("{0}-{1}.log" -f $Nom, (Get-Date -Format 'yyyyMMdd-HHmmss'))
    Start-Transcript -Path $f | Out-Null
    return $f
}

# Empreinte SHA-256 d'un fichier, comparee a son .sha256 (format sha256sum).
function Assert-Empreinte([string]$Fichier) {
    $attendu = ((Get-Content -LiteralPath "$Fichier.sha256" -Raw).Trim() -split '\s+')[0].ToLowerInvariant()
    $reel = (Get-FileHash -Algorithm SHA256 -LiteralPath $Fichier).Hash.ToLowerInvariant()
    if ($attendu -ne $reel) { Stop-Bouchaud "empreinte de $Fichier differente ($reel au lieu de $attendu)" }
    return $reel
}

# composants.txt (cle=valeur) en table.
function Get-Composants([string]$Dossier) {
    $t = @{}
    $f = Join-Path $Dossier 'composants.txt'
    if (-not (Test-Path -LiteralPath $f)) { Stop-Bouchaud "composants.txt absent dans $Dossier" }
    foreach ($l in Get-Content -LiteralPath $f) {
        if ($l -match '^([^=]+)=(.*)$') { $t[$Matches[1]] = $Matches[2] }
    }
    return $t
}

Export-ModuleMember -Function *
