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

.PARAMETER LadybirdDepuis
    Racine d'un AUTRE arbre de travail d'ou reprendre `ladybird-browser.img`.

    Un worktree fraichement cree ne contient aucun artefact Ladybird : ils ne
    sont pas suivis par Git, et ils pesent plus d'un gigaoctet. Sans cette
    option, essayer de construire une image depuis un worktree envoie
    reconstruire tout Ladybird -- une heure -- pour un lot qui ne touche que
    le noyau.

    L'image Ladybird ne depend QUE du code du navigateur. Tant qu'un lot ne
    modifie ni `tools/ladybird/` ni les services C++, celle de l'arbre voisin
    est exactement la bonne. Elle est verifiee avant d'etre reprise.

        .\tools\reference\IMAGE-TRIGKEY.ps1 -LadybirdDepuis ..\bouchaud-os
#>
[CmdletBinding()]
param(
    [switch]$QuandMeme,
    [switch]$ForceLadybird,
    [string]$LadybirdDepuis
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Set-Location $RepoRoot

function Fail([string]$Message) {
    Write-Host ""
    Write-Host "ERREUR: $Message" -ForegroundColor Red
    exit 1
}

# BOUCHAUD_LOCAL_ENV_BRDP_V1
# Le secret du banc peut vivre dans .env, ignore par Git.
# Une variable deja presente dans le processus reste prioritaire.
$EnvFile = Join-Path $RepoRoot ".env"

if (-not $env:BOUCHAUD_DEBUG_TOKEN -and
    (Test-Path -LiteralPath $EnvFile -PathType Leaf)) {

    $EnvLine = Get-Content -LiteralPath $EnvFile |
        Where-Object { $_ -match '^\s*BOUCHAUD_DEBUG_TOKEN\s*=' } |
        Select-Object -Last 1

    if ($EnvLine) {
        $Sep = $EnvLine.IndexOf("=")

        if ($Sep -ge 0) {
            $LocalToken = $EnvLine.Substring($Sep + 1).Trim()

            if (-not [string]::IsNullOrWhiteSpace($LocalToken)) {
                $env:BOUCHAUD_DEBUG_TOKEN = $LocalToken
                Write-Host (
                    "BRDP : token LAB charge depuis .env (valeur masquee, longueur={0})." `
                    -f $LocalToken.Length
                ) -ForegroundColor Green
            }
        }
    }
}

if (-not $env:BOUCHAUD_DEBUG_TOKEN) {
    Fail (
        "BOUCHAUD_DEBUG_TOKEN absent. " +
        "Cree .env depuis .env.example ou pose la variable dans l'environnement. " +
        "Construction physique refusee pour eviter une image LAB sans BRDP."
    )
}

Write-Host "BRDP LAB = ARME (secret present, valeur masquee)" -ForegroundColor Green

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

# --- 1 bis. Reprendre l'image Ladybird d'un arbre voisin ---------------------
#
# `prepare-reference-ladybird.ps1` court-circuite toute sa chaine quand
# `ladybird-browser.img` existe a la racine ET se verifie. Copier ce seul
# fichier suffit donc : ni `native-browser-m9`, ni `scenario-stage2-ladybird`,
# ni la moindre recompilation C++.
if ($LadybirdDepuis) {
    if ($ForceLadybird) {
        Fail "-LadybirdDepuis et -ForceLadybird se contredisent : l'un reprend, l'autre reconstruit."
    }
    $Voisin = $null
    try {
        $Voisin = (Resolve-Path -LiteralPath $LadybirdDepuis -ErrorAction Stop).Path
    } catch {
        Fail "arbre voisin introuvable : $LadybirdDepuis"
    }
    if ($Voisin -eq $RepoRoot) {
        Fail "-LadybirdDepuis designe cet arbre-ci : il n'y a rien a reprendre."
    }
    $Source = Join-Path $Voisin "ladybird-browser.img"
    if (-not (Test-Path -LiteralPath $Source -PathType Leaf)) {
        Fail (
            "ladybird-browser.img absent de $Voisin. Construis-la une fois " +
            "dans cet arbre-la (.\run.ps1 -Ladybird), puis reviens ici."
        )
    }
    $Cible = Join-Path $RepoRoot "ladybird-browser.img"
    $Reprendre = $true
    if (Test-Path -LiteralPath $Cible -PathType Leaf) {
        $A = (Get-FileHash -Algorithm SHA256 -LiteralPath $Source).Hash
        $B = (Get-FileHash -Algorithm SHA256 -LiteralPath $Cible).Hash
        if ($A -eq $B) {
            Write-Host "Ladybird : image deja identique a celle de $Voisin." -ForegroundColor Green
            $Reprendre = $false
        }
    }
    if ($Reprendre) {
        Write-Host "Ladybird : reprise de $Source" -ForegroundColor Cyan
        Copy-Item -LiteralPath $Source -Destination $Cible -Force
    }
    # VERIFIER AVANT DE S'EN SERVIR. Une image reprise d'un autre arbre peut
    # dater d'une generation de services que ce noyau ne sait plus lancer ;
    # le dire ici coute une seconde, le decouvrir sur la machine coute un
    # flash et un deplacement.
    & python (Join-Path $RepoRoot "tools\reference\verify-reference-ladybird-image.py") $Cible
    if ($LASTEXITCODE -ne 0) {
        Fail (
            "l'image Ladybird reprise de $Voisin ne se verifie pas. Reconstruis-la " +
            "la-bas, ou relance ici avec -ForceLadybird."
        )
    }
    Write-Host "Ladybird : image reprise et verifiee." -ForegroundColor Green
    Write-Host ""
}

# --- 2. Construction --------------------------------------------------------
$Sortie = Join-Path $RepoRoot "target\reference\bouchaud-trigkey-stage2-ladybird.img"
$Debut = Get-Date

& ".\tools\reference\build-trigkey-stage2-usb.ps1" -Output $Sortie -ForceLadybird:$ForceLadybird
if ($LASTEXITCODE -ne 0) { Fail "construction de l'image en echec" }
if (-not (Test-Path -LiteralPath $Sortie -PathType Leaf)) { Fail "image attendue absente : $Sortie" }

$Duree = [int]((Get-Date) - $Debut).TotalSeconds
$Octets = (Get-Item -LiteralPath $Sortie).Length
$Sha = (Get-FileHash -Algorithm SHA256 -LiteralPath $Sortie).Hash

# --- 2 bis. Le noyau EXACT voyage avec l'image ------------------------------
#
# Un RIP releve par la BLACKBOX ne veut rien dire sans le binaire qui l'a
# produit. Le resoudre contre un autre noyau rend une reponse -- et c'est
# presque toujours la mauvaise, parce qu'entre deux constructions l'editeur de
# liens deplace tout. Une fonction innocente prend la place de la coupable.
#
# L'ELF non strip, sa somme et les parametres de construction sont donc
# conserves A COTE de l'image, dans un manifeste que
# `tools/reference/symbolise-blackbox.py` lit et VERIFIE avant de repondre.
$Noyau = Join-Path $RepoRoot "target\x86_64-bouchaud_os_uefi\debug\bouchaud-os"
if (-not (Test-Path -LiteralPath $Noyau -PathType Leaf)) {
    $Noyau = Join-Path $RepoRoot "target\x86_64-bouchaud_os\debug\bouchaud-os"
}
$NoyauCopie = "$Sortie.kernel.elf"
$Manifeste = "$Sortie.manifeste.json"
$ShaNoyau = ""
if (Test-Path -LiteralPath $Noyau -PathType Leaf) {
    Copy-Item -LiteralPath $Noyau -Destination $NoyauCopie -Force
    $ShaNoyau = (Get-FileHash -Algorithm SHA256 -LiteralPath $NoyauCopie).Hash.ToLower()
    $Toolchain = (& rustc --version) -join ""
    $Infos = [ordered]@{
        commit = $Etiquette
        branche = $Branche
        arbre_propre = (-not [bool]$Sale)
        date = (Get-Date).ToUniversalTime().ToString("o")
        image = [ordered]@{
            chemin = $Sortie
            octets = $Octets
            sha256 = $Sha.ToLower()
        }
        noyau = [ordered]@{
            chemin = $NoyauCopie
            source = $Noyau
            sha256 = $ShaNoyau
            base = "0x8000000000"
            strip = $false
        }
        construction = [ordered]@{
            cible = "targets/x86_64-bouchaud_os_uefi.json"
            profil = "dev"
            fonctionnalites = "uefi-boot,reference-bringup,reference-desktop"
            toolchain = $Toolchain
        }
    }
    $Infos | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $Manifeste -Encoding ASCII
} else {
    Write-Host "AVERTISSEMENT: ELF noyau introuvable, pas de manifeste de symbolisation." -ForegroundColor Yellow
}

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
if ($ShaNoyau) {
    Write-Host "NOYAU ELF     = $NoyauCopie"
    Write-Host "NOYAU SHA256  = $ShaNoyau"
    Write-Host "MANIFESTE     = $Manifeste"
}
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
    "BOUCHAUD_NVME_PERSISTENCE_FIL_LANCE      le montage part dans son propre fil",
    "BOUCHAUD_NVME_GREEN                      le disque interne repond et se decrit",
    "NVME_IO_READ_ENTER                       la premiere lecture reelle entre",
    "NVME_IO_DOORBELL                         la sonnette d'entree-sortie est ecrite",
    "NVME_IO_CQE_OK                           l'achevement revient",
    "NVME_IO_COPY_END                         les octets arrivent chez l'appelant",
    "BOUCHAUD_INSTALL_SYSTEME_MONTE           la partition Bouchaud est montee",
    "BOUCHAUD_USB_HID_FIL_LANCE               l'entree quitte la boucle de trames",
    "BOUCHAUD_USB_STOCKAGE_TROUVE             points Bulk IN/OUT configures",
    "BOUCHAUD_USB_STOCKAGE_PRET               READ CAPACITY a repondu",
    "BOUCHAUD_USB_STOCKAGE_VOLUME             la cle est publiee sous la couche bloc",
    "BOUCHAUD_TRIGKEY_RTL8168_DETECTED        carte reseau vue (cable branche)",
    "BOUCHAUD_TRIGKEY_RTL8168_LINK_UP         lien Ethernet monte",
    "SMP_HANDOFF_BEFORE_STI                   etat juste AVANT la remise en service des IRQ",
    "SMP4_AP_STARTED count=15 expected=15     les quinze AP ont demarre",
    "SMP_HANDOFF_AFTER_FIRST_IRQ              la PREMIERE interruption apres la frontiere",
    "SMP_NG2_SCHEDULER                        le scheduler SMP est active",
    "[IRQ-IMPREVUES]                          parasites PIC/LAPIC et portes IDT absentes",
    "BOUCHAUD_BOOT_POINT <etape> t_ms= delta_ms=   ou part le temps d amorcage",
    "BOUCHAUD_TRIGKEY_BLACKBOX_V1 START ... com1=  ce que le demarrage avait deja produit",
    "[SYSCALL-TOP]                            a quoi un processus brule un coeur",
    "[USB-HID-POINT]                          un etat par point de terminaison HID",
    "[USB-HID-TEMOINS]                        les temoins du clavier (la LED)",
    "cpus ts_ns= cpu0=[...]                   les seize coeurs, pas quatre"
) | ForEach-Object { Write-Host "  $_" }

Write-Host ""
Write-Host "--- MARQUEURS QUI DOIVENT RESTER ABSENTS ---" -ForegroundColor Cyan
@(
    "BOUCHAUD_TAS_OOM                         le tas n'a plus rien pu servir",
    "BOUCHAUD_HEAP_LISTE_LIBRE_CORROMPUE      usage-apres-liberation dans le tas",
    "BOUCHAUD_PILE_NOYAU_DEBORDEE             pile noyau entree dans sa page de garde",
    "BOUCHAUD_NVME_HORS_SERVICE               le NVMe a ete mis en quarantaine",
    "NVME_IO_DELAI                            une commande n'a pas ete achevee a temps",
    "BOUCHAUD_NVME_PERSISTENCE_FIL_REFUSE     le fil de montage n'a pas pu etre cree",
    "NVME_IO_QUARANTAINE                      une commande abandonnee retient le tampon",
    "NVME_IO_CQE_REJETE                       achevement inconnu, perime, double ou hors domaine",
    "BOUCHAUD_USB_BULK_ECHEC                  un transfert Bulk a echoue",
    "BOUCHAUD_USB_STOCKAGE_IO_ECHEC           lecture ou ecriture refusee par la cle",
    "BOUCHAUD_USB_STOCKAGE_REINIT_ECHEC       reinitialisation Bulk-Only sans effet",
    "BOUCHAUD_USB_HID_FIL_REFUSE              l'entree est restee dans la boucle de trames",
    "BOUCHAUD_IDT_NOT_PRESENT_TEMPETE         un vecteur non servi est livre en boucle",
    "BOUCHAUD_SEGMENT_ABSENT                  #NP qui ne designe pas l'IDT : vraie faute de segment"
) | ForEach-Object { Write-Host "  $_" }

Write-Host ""
Write-Host "--- LA FRONTIERE SMP : CE QU'ON CHERCHE CE COUP-CI ---" -ForegroundColor Cyan
@(
    "Le releve du 14 septembre meurt en double faute juste apres",
    "SMP_BOOT_GUARD_EXIT, sans dire quel vecteur etait demande. Trois lignes",
    "repondent maintenant, et il faut les relever DANS CET ORDRE :",
    "",
    "  1. SMP_HANDOFF_BEFORE_STI ... pic_maitre= pic_esclave= lapic_svr=",
    "     L'etat dans lequel la premiere interruption va etre livree.",
    "",
    "  2. SMP_HANDOFF_AFTER_FIRST_IRQ vector=0xNN ...",
    "     ABSENTE = la machine est morte AVANT de servir la moindre",
    "     interruption. PRESENTE = on sait laquelle, et la frontiere est",
    "     franchie.",
    "",
    "  3. BOUCHAUD_IDT_NOT_PRESENT vector=0xNN error=... (si elle apparait)",
    "     Le processeur NOMME la porte absente. C'est la reponse que les",
    "     essais precedents ne pouvaient pas donner.",
    "",
    "Puis, sur le bureau, dans le releve periodique :",
    "  [IRQ-IMPREVUES] pic_irq7_spurious= pic_irq15_spurious= lapic_spurious=",
    "                  idt_absentes=",
    "",
    "  idt_absentes doit valoir ZERO. Un parasite PIC/LAPIC non nul est",
    "  normal sur un vrai PC -- c'est precisement ce que QEMU ne produit pas."
) | ForEach-Object { Write-Host "  $_" }

Write-Host ""
Write-Host "--- CE QUE LE LOT DU 15 SEPTEMBRE AJOUTE A RELEVER ---" -ForegroundColor Cyan
@(
    "TROIS QUESTIONS SE DECIDENT SUR CES CHIFFRES, ET SUR AUCUN RAISONNEMENT.",
    "",
    "1. LE CLAVIER",
    "   [USB-HID-POINT] ... genre=clavier ... etat= evenements= quarantaine=",
    "     etat=Running avec un TRB en attente et evenements=0 est un clavier",
    "     au repos LEGITIME. Arrete, en quarantaine ou jamais arme est un",
    "     defaut, et la ligne dit lequel.",
    "   [USB-HID-TEMOINS] poses= refuses= sondes_ep0_utiles=",
    "     poses>0 prouve que le chemin de controle atteint l'interface. Si la",
    "     LED VerrNum s'allume a l'allumage, la reponse est visible sans",
    "     ouvrir l'archive.",
    "",
    "2. LA LENTEUR DE LADYBIRD",
    "   [SYSCALL-TOP] window_ns= <nom>=<appels>/eagain=<n> ...",
    "     Le premier nom de la liste est ce que la machine fait vraiment.",
    "     Un eagain proche du nombre d'appels designe une attente active :",
    "     le processus interroge quelque chose qui ne repond jamais.",
    "   [PROC-SAMPLE] name=WebContent cpu_pct= ctx_delta=",
    "     A lire EN REGARD de la ligne precedente, pas toute seule.",
    "   BOUCHAUD_BOOT_POINT bureau / bureau-premiere-trame /",
    "   navigateur-demande / navigateur-lance",
    "     Ou part la fin du demarrage, etape par etape, en millisecondes.",
    "",
    "3. LE JOURNAL LUI-MEME",
    "   sample ... serial_perdus= serial_retard= com1=",
    "     serial_perdus=0 est la SEULE valeur qui autorise a lire l'archive",
    "     comme un recit complet. Tout le reste dit combien il en manque.",
    "   BOUCHAUD_TRIGKEY_BLACKBOX_V1 START ... arriere= capacite= com1=",
    "     arriere < capacite veut dire qu'aucune ligne anterieure a",
    "     l'enumeration USB n'a ete perdue.",
    "     com1=bus-flottant dirait que cette machine n'a pas de port serie --",
    "     et que chaque octet de journal etait jusqu'ici paye a un port",
    "     absent."
) | ForEach-Object { Write-Host "  $_" }

Write-Host ""
Write-Host "--- DEUX LIGNES DE BILAN A RELEVER AVANT D'ETEINDRE ---" -ForegroundColor Cyan
@(
    "[NVME]        lectures / ecritures / erreurs / delais / occupes / hors_service",
    "[NVME-SUIVI]  en_vol / quarantaine / echeances / tardifs / rejets",
    "",
    "  quarantaine non nul = une commande abandonnee sur echeance tient encore",
    "  le tampon de rebond ; les entrees-sorties suivantes sont REFUSEES, et",
    "  c'est voulu -- la reutiliser corromprait le tampon. Un compteur",
    "  d'erreurs a zero pendant que la quarantaine tient decrit une machine",
    "  saine qui ne lit plus rien.",
    "",
    "  rejets non nul = le controleur a envoye un achevement qui n'appartient",
    "  a aucune commande vivante. Chaque cas est nomme dans NVME_IO_CQE_REJETE."
) | ForEach-Object { Write-Host "  $_" }

Write-Host ""
Write-Host "--- SCENARIO USB PHYSIQUE (a faire APRES l'arrivee au bureau) ---" -ForegroundColor Cyan
@(
    "Ce que QEMU ne peut pas prouver : une vraie cle repond lentement, cale,",
    "renvoie des paquets courts et des STALL, et se deconnecte quand on la",
    "retire. La campagne `run_usb_stockage.sh` couvre le cas conforme ; celui-ci",
    "couvre le reste.",
    "",
    "  1. demarrer, attendre le bureau, NE RIEN brancher d'autre",
    "  2. brancher une SECONDE cle USB (pas celle de demarrage)",
    "  3. relever  BOUCHAUD_USB_STOCKAGE_TROUVE  puis  _PRET  puis  _VOLUME",
    "     -> _PRET porte blocs= et taille_bloc= : ce sont les valeurs rendues",
    "        par READ CAPACITY. Les comparer a la taille reelle de la cle.",
    "        Une cle en 4 Kio logiques rendra taille_bloc=4096 : c'est le cas",
    "        qui n'a jamais ete exerce.",
    "  4. la retirer SANS rien fermer",
    "     -> BOUCHAUD_USB_STOCKAGE_RETIRE doit apparaitre, et le systeme",
    "        continuer. Un gel ici est un defaut de deconnexion.",
    "  5. la rebrancher, sur LE MEME port",
    "     -> _TROUVE / _PRET / _VOLUME doivent revenir. Le slot peut changer.",
    "  6. la rebrancher sur un AUTRE port",
    "  7. relever [NVME-SUIVI] et les compteurs de stockage avant d'eteindre",
    "",
    "A renvoyer : les lignes BOUCHAUD_USB_* dans l'ordre, la taille reelle de",
    "la cle, et le fichier de l'enregistreur de vol s'il a ete ecrit."
) | ForEach-Object { Write-Host "  $_" }

Write-Host ""
Write-Host "--- SI L'ECRAN DE FAUTE APPARAIT, RELEVER CES LIGNES ---" -ForegroundColor Cyan
@(
    "VECTEUR / RIP / RSP     (dont NON CANONIQUE et HORS TAS NOYAU)",
    "ANNEAU                  ring 0 ou ring 3 : ce n'est pas le meme defaut",
    "POINTS FRANCHIS         dernier point de controle atteint",
    "LIENS REFUSES           non nul = liste libre du tas corrompue",
    "PILES DEBORDEES         non nul = une pile noyau est entree dans sa garde",
    "les 12 dernieres lignes serie, en bas de l'ecran",
    "le DERNIER marqueur NVME_IO_* imprime : il nomme l'etape atteinte"
) | ForEach-Object { Write-Host "  $_" }

Write-Host ""
Write-Host "--- SYMBOLISER UN RIP RELEVE ---" -ForegroundColor Cyan
Write-Host "  python3 tools/reference/symbolise-blackbox.py <RIP> --manifeste `"$Manifeste`""
Write-Host "  L'outil REFUSE de repondre si le noyau ne correspond pas au manifeste :"
Write-Host "  entre deux constructions l'editeur de liens deplace tout, et une reponse"
Write-Host "  rendue sur le mauvais binaire ressemble a une reponse juste."
Write-Host ""
Write-Host "Une photo de l'ecran de faute suffit pour le premier diagnostic ;" -ForegroundColor Yellow
Write-Host "le manifeste ci-dessus est ce qui permet d'aller jusqu'a la ligne." -ForegroundColor Yellow
Write-Host "BOUCHAUD_IMAGE_TRIGKEY_PRETE commit=$Etiquette sha256=$Sha" -ForegroundColor Green
