# Garantit que la machine a le userland de SON commit, ou explique pourquoi non.
#
#   .\tools\userland.ps1 [-Refresh] [-NoDownload] [-AllowOlder]
#
# ## Le probleme
#
# `/bo-navigateur` est un ELF unique qui embarque Qt statique, CPython, QuickJS
# et FFmpeg. Le construire demande une heure et une chaine de compilation Linux.
# Sur Windows, cela voulait dire : installer WSL, apprendre six scripts, et
# attendre — avant meme d'avoir vu le systeme. Un OS dont la premiere impression
# est « installe un autre OS d'abord » n'est pas un produit.
#
# Le userland n'a pourtant aucune raison d'etre construit par chacun : il ne
# depend que du commit source. L'integration continue le construit une fois, le
# publie en release, et ce script le recupere.
#
# ## La regle qui gouverne ce fichier
#
# **Jamais un userland d'un autre commit sans le dire.** Une image d'un autre
# jour ne se signale pas : elle demarre, et elle se comporte comme le systeme
# d'alors — un binaire qui ignore le dernier appel systeme, un moteur qui ignore
# le dernier protocole. La panne qui suit accuse le code source, qui n'y est pour
# rien, et l'on cherche des heures du mauvais cote.
#
# Trois verrous, donc : le manifeste doit porter **ce** commit, l'empreinte
# SHA-256 doit correspondre a l'image telechargee, et une image plus ancienne
# demande `-AllowOlder` **et** dit de combien elle est en retard.
#
# ## Ce que ce script n'est pas
#
# Une dependance a Linux au moment de l'execution. Bouchaud OS reste bare-metal :
# ce qui est telecharge est un disque de donnees contenant des ELF statiques,
# construits par compilation croisee, exactement comme un firmware. Aucun noyau
# Linux n'intervient a l'execution.

param(
  [string]$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")),
  # Retelecharge meme si l'image locale est valide.
  [switch]$Refresh,
  # N'essaie jamais de telecharger. Pour booter le noyau seul, volontairement.
  [switch]$NoDownload,
  # Accepte le userland d'un ancetre du commit courant, faute d'exact. Le script
  # dit alors precisement lequel et de combien de commits il est en retard.
  [switch]$AllowOlder,
  # Combien d'ancetres explorer avec -AllowOlder. Au-dela, le decalage est trop
  # grand pour qu'accepter l'image rende service.
  [int]$MaxAncetres = 40
)

$ErrorActionPreference = "Stop"
# Invoke-WebRequest affiche une barre de progression qui, sur Windows
# PowerShell 5.1, coute plus de temps que le telechargement lui-meme.
$ProgressPreference = "SilentlyContinue"
# PowerShell 5.1 negocie encore TLS 1.0 par defaut ; github.com le refuse.
try {
  [Net.ServicePointManager]::SecurityProtocol =
    [Net.SecurityProtocolType]::Tls12 -bor [Net.ServicePointManager]::SecurityProtocol
} catch {}

$Dossier  = Join-Path $RepoRoot "tools\userland"
$Image    = Join-Path $Dossier "userland.img"
$Manifest = Join-Path $Dossier "userland-manifest.json"

function Info($t)    { Write-Host $t -ForegroundColor DarkCyan }
function Bon($t)     { Write-Host $t -ForegroundColor Green }
function Avertis($t) { Write-Host $t -ForegroundColor DarkYellow }
function Rouge($t)   { Write-Host $t -ForegroundColor Red }

function Get-DepotSlug {
  # `owner/repo`, depuis l'origine git. Surchargeable : un miroir interne ou un
  # fork n'ont pas de raison d'etre devines.
  if ($env:BO_USERLAND_REPO) { return $env:BO_USERLAND_REPO }
  $url = (& git -C $RepoRoot remote get-url origin 2>$null | Out-String).Trim()
  if ([string]::IsNullOrWhiteSpace($url)) { return $null }
  if ($url -match 'github\.com[:/](?<slug>[^/]+/[^/]+?)(\.git)?$') {
    return $Matches['slug']
  }
  return $null
}

function Test-Manifeste($chemin, $commitAttendu) {
  # Rend le manifeste s'il decrit bien l'image locale ET le commit demande.
  if (-not (Test-Path $chemin)) { return $null }
  try { $m = Get-Content $chemin -Raw | ConvertFrom-Json } catch { return $null }
  if (-not (Test-Path $Image)) { return $null }
  if ($commitAttendu -and $m.git_commit -ne $commitAttendu) { return $null }
  $somme = (Get-FileHash $Image -Algorithm SHA256).Hash.ToLower()
  if ($somme -ne ("" + $m.sha256).ToLower()) {
    Avertis "userland: l'image locale ne correspond pas a son manifeste (SHA-256)."
    return $null
  }
  return $m
}

function Recupere($slug, $commit) {
  # Rend $true si le userland de ce commit a ete recupere et verifie.
  $base = "https://github.com/$slug/releases/download/userland-$commit"
  $tmpM = [System.IO.Path]::GetTempFileName()
  $tmpI = [System.IO.Path]::GetTempFileName()
  try {
    try {
      Invoke-WebRequest -Uri "$base/userland-manifest.json" -OutFile $tmpM `
        -UseBasicParsing -ErrorAction Stop
    } catch {
      return $false      # pas de release pour ce commit : cas normal, silencieux
    }
    $m = Get-Content $tmpM -Raw | ConvertFrom-Json

    # Le manifeste est verifie **avant** de tirer les dizaines de mebioctets de
    # l'image : un manifeste qui annonce un autre commit rend le telechargement
    # inutile, et le faire quand meme donnerait envie de s'en contenter.
    if ($m.git_commit -ne $commit) {
      Rouge "userland: la release $commit contient un manifeste pour $($m.git_commit) — refuse."
      return $false
    }

    $mo = [math]::Round($m.image_size / 1MB, 1)
    Info "userland: telechargement de $mo Mo (Qt $($m.qt_version), Python $($m.python_version), FFmpeg $($m.ffmpeg_version))"
    Invoke-WebRequest -Uri "$base/userland.img" -OutFile $tmpI `
      -UseBasicParsing -ErrorAction Stop

    $somme = (Get-FileHash $tmpI -Algorithm SHA256).Hash.ToLower()
    if ($somme -ne ("" + $m.sha256).ToLower()) {
      Rouge "userland: empreinte SHA-256 incorrecte — image jetee."
      Rouge "  attendu $($m.sha256)"
      Rouge "  obtenu  $somme"
      return $false
    }
    $taille = (Get-Item $tmpI).Length
    if ($taille -ne $m.image_size) {
      Rouge "userland: taille inattendue ($taille au lieu de $($m.image_size)) — image jetee."
      return $false
    }

    New-Item -ItemType Directory -Force -Path $Dossier | Out-Null
    Move-Item -Force $tmpI $Image
    Move-Item -Force $tmpM $Manifest
    return $true
  } finally {
    foreach ($f in @($tmpM, $tmpI)) {
      if (Test-Path $f) { Remove-Item -Force $f -ErrorAction SilentlyContinue }
    }
  }
}

# --- Deroulement --------------------------------------------------------------

Write-Host "=== Userland ===" -ForegroundColor Cyan

$commit = (& git -C $RepoRoot rev-parse HEAD 2>$null | Out-String).Trim()
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($commit)) {
  Avertis "pas de commit git lisible : le userland local est pris tel quel."
  if (Test-Path $Image) { Bon "userland: $Image" } else { Avertis "userland: absent." }
  exit 0
}

if (-not $Refresh) {
  $m = Test-Manifeste $Manifest $commit
  if ($m) {
    $mo = [math]::Round($m.image_size / 1MB, 1)
    Bon "userland: deja la, pour ce commit ($mo Mo, Qt $($m.qt_version))"
    exit 0
  }
}

if ($NoDownload) {
  Avertis "userland: -NoUserlandDownload — la machine demarrera sans userland."
  Avertis "  ni Qt, ni Python, ni navigateur : le bureau annoncera /bo-navigateur absent."
  exit 0
}

$slug = Get-DepotSlug
if (-not $slug) {
  Avertis "userland: depot GitHub d'origine inconnu — rien a telecharger."
  Avertis "  forcer avec :  `$env:BO_USERLAND_REPO = 'proprietaire/depot'"
  exit 0
}

Info "userland: recherche pour $($commit.Substring(0,12)) dans $slug"
if (Recupere $slug $commit) {
  Bon "userland: recupere et verifie pour ce commit."
  exit 0
}

# Rien pour ce commit exactement. C'est le cas normal sur une branche de
# travail : l'integration continue ne publie que pour `main`.
if ($AllowOlder) {
  Info "userland: aucun pour ce commit — recherche chez ses ancetres"
  $ancetres = & git -C $RepoRoot rev-list --max-count=$MaxAncetres HEAD
  $rang = 0
  foreach ($a in $ancetres) {
    $rang++
    if ($a -eq $commit) { continue }
    if (Recupere $slug $a) {
      Avertis ""
      Avertis "userland: image d'un AUTRE commit, acceptee parce que -AllowOlder."
      Avertis "  commit du userland : $a"
      Avertis "  commit de la source: $commit"
      Avertis "  ecart              : $rang commit(s)"
      Avertis "  Un ecart entre le noyau et son userland ne se signale pas tout"
      Avertis "  seul : si quelque chose se comporte mal, soupconner cet ecart"
      Avertis "  avant de soupconner le code."
      exit 0
    }
  }
  Rouge "userland: aucun userland publie parmi les $MaxAncetres derniers commits."
} else {
  Rouge "userland: aucun userland publie pour le commit $($commit.Substring(0,12))."
}

Write-Host ""
Write-Host "Trois facons d'avancer :" -ForegroundColor Yellow
Write-Host "  1. se placer sur un commit publie :   git checkout main; git pull"
Write-Host "  2. accepter le userland d'un ancetre : .\run.ps1 -AllowOlderUserland"
Write-Host "  3. demarrer le noyau seul :           .\run.ps1 -NoUserlandDownload"
Write-Host ""
Write-Host "L'integration continue publie un userland par commit de main"
Write-Host "(workflow 'userland'). Une branche de travail n'en a pas tant que"
Write-Host "son build n'a pas tourne." -ForegroundColor DarkGray
# Non bloquant : le noyau demarre tres bien sans userland, et refuser de lancer
# QEMU pour cela reviendrait a cacher le systeme parce qu'il lui manque une
# application.
exit 0
