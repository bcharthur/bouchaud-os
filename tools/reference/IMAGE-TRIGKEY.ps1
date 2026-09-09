<#
.SYNOPSIS
    Construit l'image USB du TRIGKEY correspondant EXACTEMENT au HEAD courant,
    publie son SHA256, et rappelle les marqueurs attendus.

.DESCRIPTION
    Une seule commande, depuis la racine du depot :

        powershell -ExecutionPolicy Bypass -File .\tools\reference\IMAGE-TRIGKEY.ps1

    Le point de cette commande est la CORRESPONDANCE : une image construite a
    partir d'un arbre modifie ne correspond a aucun commit, et le releve d'un
    essai physique fait sur une telle image ne peut etre rattache a rien. Le
    script refuse donc de construire sur un arbre sale, sauf `-QuandMeme`, et
    publie dans tous les cas le commit exact.

.PARAMETER QuandMeme
    Construit malgre des modifications locales. L'image est alors marquee
    `+sale` : elle ne correspond a aucun commit, et le dire vaut mieux que de
    laisser croire le contraire.

.PARAMETER ForceLadybird
    Reconstruit le disque memoire Ladybird au lieu de reprendre celui qui est
    deja la. Long.
#>
[CmdletBinding()]
param(
    [switch]$QuandMeme,
    [switch]$ForceLadybird
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot

function Fail([string]$Message) {
    Write-Host ""
    Write-Host "ERREUR: $Message" -ForegroundColor Red
    exit 1
}

Write-Host "=== BOUCHAUD OS - IMAGE USB TRIGKEY ===" -ForegroundColor Cyan
Write-Host ""

# --- 1. A quel commit cette image correspondra-t-elle ? ---------------------
$Commit = (& git rev-parse HEAD 2>$null)
if ($LASTEXITCODE -ne 0) { Fail "git indisponible ou depot introuvable" }
$Commit = $Commit.Trim()
$Branche = (& git rev-parse --abbrev-ref HEAD).Trim()
$Sale = (& git status --porcelain) | Where-Object { $_ -ne "" }

if ($Sale) {
    Write-Host "Arbre de travail MODIFIE :" -ForegroundColor Yellow
    $Sale | Select-Object -First 20 | ForEach-Object { Write-Host "  $_" }
    if (-not $QuandMeme) {
        Write-Host ""
        Write-Host "  - soit committe (ou remise) ces modifications, puis relance ;"
        Write-Host "  - soit relance avec -QuandMeme si l'essai porte sur elles."
        Fail "l'image ne correspondrait a aucun commit"
    }
    Write-Host ""
    Write-Host "-QuandMeme : on construit, mais l'image ne correspond a AUCUN commit." -ForegroundColor Yellow
    $Etiquette = "$Commit+sale"
} else {
    $Etiquette = $Commit
}

Write-Host "BRANCHE = $Branche"
Write-Host "COMMIT  = $Etiquette"
Write-Host ""

# --- 2. Construction --------------------------------------------------------
$Sortie = Join-Path $RepoRoot "target\reference\bouchaud-trigkey-stage2-ladybird.img"
$Debut = Get-Date

& ".\tools\reference\build-trigkey-stage2-usb.ps1" -Output $Sortie -ForceLadybird:$ForceLadybird
if ($LASTEXITCODE -ne 0) { Fail "construction de l'image en echec" }
if (-not (Test-Path -LiteralPath $Sortie -PathType Leaf)) { Fail "image attendue absente : $Sortie" }

$Duree = [int]((Get-Date) - $Debut).TotalSeconds
$Octets = (Get-Item -LiteralPath $Sortie).Length
$Sha = (Get-FileHash -Algorithm SHA256 -LiteralPath $Sortie).Hash

# --- 3. Ce qu'il faut savoir avant de flasher --------------------------------
Write-Host ""
Write-Host "=====================================================================" -ForegroundColor Green
Write-Host " IMAGE PRETE" -ForegroundColor Green
Write-Host "=====================================================================" -ForegroundColor Green
Write-Host "COMMIT        = $Etiquette"
Write-Host "IMAGE         = $Sortie"
Write-Host "OCTETS        = $Octets  ($([math]::Round($Octets / 1MB, 1)) Mio)"
Write-Host "SHA256        = $Sha" -ForegroundColor Green
Write-Host "DUREE         = ${Duree}s"
Write-Host ""
Write-Host "FLASH : Rufus -> Selection de demarrage -> cette image -> mode DD." -ForegroundColor Yellow
Write-Host "        Secure Boot desactive, Boot Override sur la cle UEFI." -ForegroundColor Yellow
Write-Host "        NE PAS choisir le NVMe interne comme cible dans Rufus." -ForegroundColor Yellow
Write-Host ""

Write-Host "--- MARQUEURS ATTENDUS (console serie / diagnostic) ---" -ForegroundColor Cyan
@(
    "BOUCHAUD_TRIGKEY_XHCI_PRESENT            controleur USB vu",
    "BOUCHAUD_TRIGKEY_LADYBIRD_RAMDISK_OK     disque memoire Ladybird monte",
    "BOUCHAUD_STAGE2_LADYBIRD_RUNTIME_OK      execution Ladybird prete",
    "BOUCHAUD_TAS_PAGES_PRET                  compagnon du tas configure",
    "BOUCHAUD_TAS_BACKING compagnon=pages     le backing normal est le compagnon",
    "BOUCHAUD_HEAP_ARENE_CACHES_VIDES         basculement d'arene propre",
    "BOUCHAUD_NVME_PERSISTENCE_DEFERRED       la persistance ne bloque plus le demarrage",
    "BOUCHAUD_TRIGKEY_RTL8168_DETECTED        carte reseau vue (cable branche)",
    "BOUCHAUD_TRIGKEY_RTL8168_LINK_UP         lien Ethernet monte"
) | ForEach-Object { Write-Host "  $_" }

Write-Host ""
Write-Host "--- MARQUEURS QUI DOIVENT RESTER ABSENTS ---" -ForegroundColor Cyan
@(
    "BOUCHAUD_TAS_OOM                         le tas n'a plus rien pu servir",
    "BOUCHAUD_HEAP_LISTE_LIBRE_CORROMPUE      usage-apres-liberation dans le tas",
    "BOUCHAUD_PILE_NOYAU_DEBORDEE             pile noyau entree dans sa page de garde",
    "BOUCHAUD_NVME_HORS_SERVICE               le NVMe a ete mis en quarantaine"
) | ForEach-Object { Write-Host "  $_" }

Write-Host ""
Write-Host "--- SI L'ECRAN DE FAUTE APPARAIT, RELEVER CES LIGNES ---" -ForegroundColor Cyan
@(
    "VECTEUR / RIP / RSP     (dont NON CANONIQUE et HORS TAS NOYAU)",
    "ANNEAU                  ring 0 ou ring 3 : ce n'est pas le meme defaut",
    "POINTS FRANCHIS         dernier point de controle atteint",
    "LIENS REFUSES           non nul = liste libre du tas corrompue",
    "PILES DEBORDEES         non nul = une pile noyau est entree dans sa garde",
    "les 12 dernieres lignes serie, en bas de l'ecran"
) | ForEach-Object { Write-Host "  $_" }

Write-Host ""
Write-Host "Une photo de l'ecran de faute suffit : tout y est." -ForegroundColor Yellow
Write-Host "BOUCHAUD_IMAGE_TRIGKEY_PRETE commit=$Etiquette sha256=$Sha" -ForegroundColor Green
