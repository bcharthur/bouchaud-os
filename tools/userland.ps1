# Garantit que la machine a le userland de SON commit, ou explique pourquoi non.
#
#   .\tools\userland.ps1 [-Refresh] [-NoDownload] [-AllowOlder]
#
# ## Le probleme
#
# `/bo-navigateur` est un ELF unique qui embarque Qt statique, CPython, QuickJS
# et FFmpeg. Le construire demande une heure et une chaine de compilation Linux.
# Sur Windows, cela voulait dire : installer WSL, apprendre six scripts, et
# attendre - avant meme d'avoir vu le systeme. Un OS dont la premiere impression
# est "installe un autre OS d'abord" n'est pas un produit.
#
# Le userland n'a pourtant aucune raison d'etre construit par chacun : il ne
# depend que du commit source. L'integration continue le construit une fois, le
# publie en release, et ce script le recupere.
#
# ## La regle qui gouverne ce fichier
#
# **Jamais un userland d'un autre commit sans le dire.** Une image d'un autre
# jour ne se signale pas : elle demarre, et elle se comporte comme le systeme
# d'alors - un binaire qui ignore le dernier appel systeme, un moteur qui ignore
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

# Ce qu'une tentative de telechargement a donne. Trois issues, et il faut
# vraiment les trois : "absent" est un fait ordinaire, "erreur" est un incident
# qu'il faut montrer, et les confondre a coute une soiree a l'utilisateur.
#
# La premiere version rendait $false pour n'importe quel echec. Le message
# disait alors "aucun userland publie pour ce commit" alors que la release
# existait, avec ses deux fichiers, aux adresses exactes que le script
# construit. Le script affirmait quelque chose qu'il n'avait aucun moyen de
# savoir, et envoyait chercher du cote de l'integration continue un defaut qui
# etait du cote du reseau.
$ABSENT = "absent"
$OBTENU = "obtenu"
$ERREUR = "erreur"

function Get-Statut($erreur) {
  # Le code HTTP d'une exception d'Invoke-WebRequest, ou 0 s'il n'y en a pas.
  # Pas de code veut dire que la requete n'a jamais abouti : nom non resolu,
  # poignee de main TLS refusee, mandataire qui coupe. C'est precisement la
  # distinction qui manquait.
  try {
    $reponse = $erreur.Exception.Response
    if ($reponse -and $reponse.StatusCode) { return [int]$reponse.StatusCode }
  } catch {}
  return 0
}

function Telecharge($url, $sortie) {
  # Rend "obtenu", "absent" (404), ou "erreur" en ayant dit pourquoi.
  #
  # Deux transports, et le second n'est pas un luxe : "Invoke-WebRequest" passe
  # par la pile TLS de .NET Framework, dont la configuration par defaut varie
  # d'un poste Windows a l'autre : versions de TLS activees, mandataire du
  # systeme, inspection par un antivirus. "curl.exe" est livre avec Windows 10
  # depuis 1803 et a sa propre pile ; quand l'un echoue sans raison lisible,
  # l'autre passe souvent.
  $premier = ""
  try {
    Invoke-WebRequest -Uri $url -OutFile $sortie -UseBasicParsing -ErrorAction Stop
    return $OBTENU
  } catch {
    $statut = Get-Statut $_
    if ($statut -eq 404) { return $ABSENT }
    $premier = $_.Exception.Message
  }

  $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
  if ($curl) {
    # -f : rend un code d'erreur sur un statut HTTP superieur ou egal a 400 au
    # lieu d'ecrire la page d'erreur dans le fichier. -L : suit la redirection
    # vers le stockage des artefacts, ou GitHub renvoie toujours.
    $code = & $curl.Source -sS -L -f -o $sortie -w "%{http_code}" $url 2>&1
    if ($LASTEXITCODE -eq 0) { return $OBTENU }
    if ("$code" -match "404") { return $ABSENT }
    Rouge "userland: le telechargement a echoue."
    Rouge "  adresse    : $url"
    Rouge "  PowerShell : $premier"
    Rouge "  curl       : $code"
    return $ERREUR
  }

  Rouge "userland: le telechargement a echoue."
  Rouge "  adresse    : $url"
  Rouge "  PowerShell : $premier"
  return $ERREUR
}

function Recupere($slug, $commit) {
  # Rend "obtenu", "absent" ou "erreur" pour le userland de ce commit.
  $base = "https://github.com/$slug/releases/download/userland-$commit"
  $tmpM = [System.IO.Path]::GetTempFileName()
  $tmpI = [System.IO.Path]::GetTempFileName()
  try {
    $etat = Telecharge "$base/userland-manifest.json" $tmpM
    if ($etat -ne $OBTENU) { return $etat }
    $m = Get-Content $tmpM -Raw | ConvertFrom-Json

    # Le manifeste est verifie **avant** de tirer les dizaines de mebioctets de
    # l'image : un manifeste qui annonce un autre commit rend le telechargement
    # inutile, et le faire quand meme donnerait envie de s'en contenter.
    if ($m.git_commit -ne $commit) {
      Rouge "userland: la release $commit contient un manifeste pour $($m.git_commit) - refuse."
      return $ERREUR
    }

    $mo = [math]::Round($m.image_size / 1MB, 1)
    Info "userland: telechargement de $mo Mo (Qt $($m.qt_version), Python $($m.python_version), FFmpeg $($m.ffmpeg_version))"
    $etat = Telecharge "$base/userland.img" $tmpI
    if ($etat -ne $OBTENU) {
      # Le manifeste etait la et l'image ne l'est pas : ce n'est pas une release
      # absente, c'est une release incomplete ou un telechargement coupe.
      if ($etat -eq $ABSENT) {
        Rouge "userland: la release existe mais son image manque."
        Rouge "  $base/userland.img"
      }
      return $ERREUR
    }

    $somme = (Get-FileHash $tmpI -Algorithm SHA256).Hash.ToLower()
    if ($somme -ne ("" + $m.sha256).ToLower()) {
      Rouge "userland: empreinte SHA-256 incorrecte - image jetee."
      Rouge "  attendu $($m.sha256)"
      Rouge "  obtenu  $somme"
      return $ERREUR
    }
    $taille = (Get-Item $tmpI).Length
    if ($taille -ne $m.image_size) {
      Rouge "userland: taille inattendue ($taille au lieu de $($m.image_size)) - image jetee."
      return $ERREUR
    }

    New-Item -ItemType Directory -Force -Path $Dossier | Out-Null
    Move-Item -Force $tmpI $Image
    Move-Item -Force $tmpM $Manifest
    return $OBTENU
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
  Avertis "userland: -NoUserlandDownload - la machine demarrera sans userland."
  Avertis "  ni Qt, ni Python, ni navigateur : le bureau annoncera /bo-navigateur absent."
  exit 0
}

$slug = Get-DepotSlug
if (-not $slug) {
  Avertis "userland: depot GitHub d'origine inconnu - rien a telecharger."
  Avertis "  forcer avec :  `$env:BO_USERLAND_REPO = 'proprietaire/depot'"
  exit 0
}

Info "userland: recherche pour $($commit.Substring(0,12)) dans $slug"
$etat = Recupere $slug $commit
if ($etat -eq $OBTENU) {
  Bon "userland: recupere et verifie pour ce commit."
  exit 0
}

# Un echec de transport n'est pas une absence, et les deux ne se reparent pas
# de la meme facon. Le dire separement est tout l'objet de cette distinction :
# renvoyer quelqu'un vers l'integration continue quand c'est son reseau qui
# coupe lui fait chercher des heures du mauvais cote.
if ($etat -eq $ERREUR) {
  Write-Host ""
  Write-Host "La release existe peut-etre : c'est la requete qui a echoue." -ForegroundColor Yellow
  Write-Host "  Verifier a la main dans un navigateur :"
  Write-Host "  https://github.com/$slug/releases/tag/userland-$commit"
  Write-Host ""
  Write-Host "  Si la page existe, telecharger ses deux fichiers dans"
  Write-Host "  tools\userland\ : le script les reconnaitra au prochain demarrage."
  Write-Host "  Causes habituelles : mandataire d'entreprise, antivirus qui"
  Write-Host "  inspecte le TLS, ou TLS 1.2 desactive dans .NET Framework."
  Write-Host ""
  Write-Host "  Pour demarrer sans userland :  .\run.ps1 -NoUserlandDownload"
  exit 0
}

# Rien pour ce commit exactement. C'est le cas normal sur une branche de
# travail : l'integration continue ne publie que pour `main`.
if ($AllowOlder) {
  Info "userland: aucun pour ce commit - recherche chez ses ancetres"
  $ancetres = & git -C $RepoRoot rev-list --max-count=$MaxAncetres HEAD
  $rang = 0
  foreach ($a in $ancetres) {
    $rang++
    if ($a -eq $commit) { continue }
    if ((Recupere $slug $a) -eq $OBTENU) {
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
  Rouge "  (verifie : la page de release repond 404)"
}

Write-Host ""
Write-Host "Trois facons d'avancer :" -ForegroundColor Yellow
Write-Host "  1. se placer sur un commit publie :   git checkout main; git pull"
Write-Host "  2. accepter le userland d'un ancetre : .\run.ps1 -AllowOlderUserland"
Write-Host "  3. demarrer le noyau seul :           .\run.ps1 -NoUserlandDownload"
Write-Host ""
Write-Host "L'integration continue publie un userland par commit de main"
Write-Host "(workflow 'userland'). Une branche de travail n'en a pas tant que"
Write-Host "son build n'a pas tourne, et un merge tout frais attend le sien."
Write-Host "  suivi : https://github.com/$slug/actions/workflows/userland.yml" -ForegroundColor DarkGray
# Non bloquant : le noyau demarre tres bien sans userland, et refuser de lancer
# QEMU pour cela reviendrait a cacher le systeme parce qu'il lui manque une
# application.
exit 0
