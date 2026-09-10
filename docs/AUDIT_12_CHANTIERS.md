# Audit des douze chantiers centraux

Date : 8 septembre 2026. Base : `claude/bouchaud-os-modernization-f9zj01`,
apres l'integration NVMe / GPT / ESP / installateur.

## Comment lire ce document

**Aucun etat n'est declaratif.** Chaque ligne renvoie a un artefact qu'on peut
executer : un test hote, un garde-fou, un journal de machine. Ce qui n'a pas de
preuve executable porte 🔵 ou ⚪, jamais ✅.

| Marque | Ce que cela veut dire |
|---|---|
| ✅ | L'architecture cible est utilisee **par defaut**, et une preuve executable echoue si elle cesse de l'etre. |
| 🟡 | Le code existe, tourne, et est couvert sur l'hote. La preuve sur materiel ou sous QEMU manque. |
| 🔵 | Chantier engage : une tranche verticale existe, le chemin par defaut reste l'ancien. |
| ⚪ | Rien de ce qui compte n'est ecrit. |

L'inventaire actuel : **68 garde-fous**, **59 suites de tests hote Rust**,
5 suites C++, 12 tests de fiabilite Python, ~88 000 lignes de noyau.

## Mesures — recalculees sur le code, jamais recopiees

Ce bloc est produit par `tools/mesure-chantiers.py`. Il n'est pas ecrit a la
main, et `--verifie` le compare a l'arbre dans la barriere d'architecture : un
document dont les chiffres ont peri fait echouer la CI.

C'est la reponse a la facon dont ce document avait peri une premiere fois. Il
affirmait « futex reste sous BKL » et « pas de W^X » plusieurs commits apres que
le code eut dit l'inverse. Un document faux est pire qu'un document absent : on
lui fait confiance.

<!-- MESURE-CHANTIERS:DEBUT -->
| Mesure | Valeur |
|---|---|
| Appels systeme aiguilles | **159** |
| Appels hors gros verrou | **81** |
| Appels encore sous gros verrou | **78** |
| Fichiers portant un point sur de preemption | **4** |
| Garde-fous d'architecture | **59** |
| Suites de test hote | **66** |
| W^X applique au chargement ELF | **oui** |
| Canari de pile noyau | **oui** |
<!-- MESURE-CHANTIERS:FIN -->

Regenerer : `python3 tools/mesure-chantiers.py --ecris`

## Le double fault physique : ce qui est prouve, et ce qui ne l'est pas

La BLACKBOX a capture, sur le TRIGKEY :

    DOUBLE FAULT  vector=8  cpu=0  task=desktop  RIP=0x80012ed45c

Le dernier marqueur normal avant la faute est
`BOUCHAUD_NVME_PERSISTENCE_PROBE_BEGIN`. Le chemin execute est donc :

    desktop -> montage differe -> monte_le_systeme_installe -> gpt::lit_table
            -> Support::lit(LBA 1) -> couche bloc -> PiloteNvme
            -> premiere entree-sortie NVMe reelle

### Ce qui est prouve

Un debordement de pile CERTAIN a ete trouve et corrige ailleurs :
`net::transport::tcp::fetch` posait sa file de retransmission -- environ
quatre-vingt-treize kibioctets -- sur une pile noyau de soixante-quatre. La
mesure est reproductible (`tools/mesure-pile-noyau.py`) : trame de 99 320
octets, pire chaine de 119 888 octets, pile utilisable de 61 440.

Ce defaut est reel et sa correction etait necessaire. Il aurait produit un
double fault des le premier telechargement, donc des le premier chargement de
page dans Ladybird.

### Ce qui n'est PAS prouve

**Il n'explique pas le double fault capture.** Le chemin execute au moment de
la faute etait celui du stockage, pas celui du reseau : `tcp::fetch` n'y
apparait a aucun etage. La chaine du montage mesure 10 592 octets, tres en
deca de la pile disponible -- ce n'est pas un debordement.

Le defaut physique NVMe reste donc **ouvert**. Ce qui a ete fait autour de lui :

  * l'attente d'entree-sortie a quitte l'IRQ-off (elle bloquait la machine
    entiere jusqu'a deux secondes par commande) ;
  * huit marqueurs `NVME_IO_*` publient chaque etape AVANT de l'executer, donc
    le dernier marqueur imprime nommera l'etape atteinte ;
  * le pilote est desormais mis en service quel que soit le micrologiciel, et
    un scenario QEMU l'exerce avec un vrai disque et une vraie table GPT.

La confirmation demande un essai sur le TRIGKEY, avec l'image et le manifeste
produits par `tools/reference/IMAGE-TRIGKEY.ps1`.

### Pourquoi une adresse ne se resout pas contre le HEAD

`0x80012ed45c` resolu contre deux constructions differentes de ce depot donne
`keyboard::read_into` dans l'une et `shell::commands::find_rec` dans l'autre.
Aucune des deux n'est la reponse : entre deux commits l'editeur de liens
deplace tout, et une fonction innocente prend la place de la coupable.

`tools/reference/symbolise-blackbox.py` REFUSE de repondre si la somme du noyau
qu'on lui donne ne correspond pas au manifeste ecrit a la construction de
l'image. Une reponse rendue est une reponse sur le bon binaire, ou il n'y a pas
de reponse.

## Tableau d'ensemble

| # | Chantier | Etat | Ce qui manque, en une phrase |
|---|---|---:|---|
| 1 | BKL → noyau concurrent | 🔵 | Voir le bloc mesure ci-dessus. `futex`, la famille boucle d'evenements et le sommeil sont sortis ; les **sockets**, `openat`/coeur FS, `ioctl`, les signaux, `clone` et `execve` restent. |
| 2 | Scheduler NG + preemption | 🔵 | Preemption depuis l'IRQ seulement ; pas de points surs, pas de tickless. |
| 3 | Memoire NG | 🔵 | Compagnon pour le DMA ; `LockedHeap` reste le fond du tas noyau, pas de slab. |
| 4 | Graphique NG / compositeur ring 3 | 🔵 | Le contrat existe et le compositeur noyau reste le chemin par defaut. |
| 5 | Systeme de fichiers + E/S moderne | 🟡 | Commit A/B et barriere reelle ; pas d'extents, pas d'E/S asynchrone. |
| 6 | Architecture de securite | 🟡 | Mots de passe sales et haches, profils separes, **W^X applique** (`security/wx.rs`, refuse au chargement ELF et dans `mmap`/`mprotect`) ; pas d'ASLR, pas de sandbox M14. |
| 7 | ABI Bouchaud + IPC natif | 🟡 | Les primitives existent ; Linux reste la personnalite dominante. |
| 8 | Ladybird comme produit | 🔵 | Un renderer, pas de sandbox, pas de WPT. |
| 9 | Reseau NG | 🔵 | Retransmission et RTO prouves ; pas d'IPv6, pas de zero-copie. |
| 10 | Plateforme materielle de reference | 🟡 | **Le plus avance de tous** : UEFI, xHCI (concentrateurs traverses), NVMe, installation. Pas d'audio HDA, pas de Wi-Fi, pas de suspend. |
| 11 | Fiabilite / CI / release | 🔵 | **`main` n'est pas protege** ; fuzzing sur un seul objet ; budgets non tenus faute de campagne. |
| 12 | Polish produit | ⚪ | Ni HiDPI, ni IME, ni accessibilite, ni glisser-deposer, ni mise a jour atomique. |

---

## 1 — BKL → noyau reellement concurrent 🔵

**Ce qui existe.** Le gros verrou est passe de dizaines de sites a **15
acquisitions**, toutes attribuees a un domaine nomme. Six domaines sont
declares SORTIS et le restent sous garde-fou : `Fs`, `Ordonnanceur`,
`Readiness`, `RegistreProcessus`, `VerrouEnregistrement`, `Vfs`. L'ordre de
verrouillage est ecrit (`docs/architecture/SMP_LOCK_ORDER.md`) et 78 usages
d'etat par CPU existent.

*Preuve :* `tools/verifie-domaines-bkl.py`, `tools/verifie-portee-sans-commutation.py`,
`tools/fs/test_commit_crash.rs`, budget `sites_bkl_par_domaine`.

**Ce que cette session a change.** Dix-sept appels systeme de plus ont quitte
le gros verrou : 44 liberes au depart, **61 sur 165** maintenant. Les plus
chauds y sont passes -- `mmap` (chaque arene glibc, chaque `dlopen`, chaque
pile de fil), `close`, `fstat` (chaque `fopen`), `arch_prctl` (chaque fil
cree), `lseek`, `dup`, `sched_yield`.

`sched_yield` etait le cas absurde : il prenait le verrou, puis `schedule()`
le RELACHAIT pour commuter et le reprenait au retour -- une acquisition
globale, une liberation et une reacquisition pour un appel dont tout l'objet
est de rendre la main.

Le point qui a debloque `mmap` merite d'etre retenu : il appelle
`peuple_a_la_demande`, qui est le gestionnaire de FAUTE DE PAGE. Une faute
peut survenir a tout instant, y compris pendant qu'un autre coeur tient le
verrou. S'il en avait besoin, le systeme serait deja casse.

**Ce qui manque.** Les sockets (`recvfrom`, `sendto`, `recvmsg`, `sendmsg`),
`futex`, `openat`, `ioctl`, `execve`, `clone`.

Un cas merite d'etre nomme, parce qu'il aurait ete libere par ressemblance :
`recvfrom` sur une paire de sockets TESTE si le tampon est vide, puis entre
dans `sys_read` -- lequel est deja libere. Ce qui rend ces deux etapes
atomiques a plusieurs coeurs est le gros verrou, et rien d'autre. Le liberer
sans porter l'intention « non bloquant » jusque dans `sys_read` ferait bloquer
DEUX SECONDES une lecture declaree non bloquante, et seulement sous charge.
La dependance est maintenant ecrite dans `net.rs`, a l'endroit ou elle vit. Il n'y a toujours pas de
lockdep runtime : l'ordre est verifie par lecture de source, pas par le noyau
qui tourne. Le but final -- « le chemin normal n'en a plus besoin » -- n'est
pas atteint.

**Ce qui bloque.** Rien d'architectural. Chaque appel restant demande son
propre audit, et un appel libere par optimisme ne se voit ni a la compilation
ni au boot : il se voit un jour, sous charge, a quatre coeurs, sous la forme
d'une corruption qu'on ne saura pas relier a sa cause.

## 2 — Scheduler NG + preemption noyau 🔵

**Ce qui existe.** Runqueue O(1) a deux bandes, sans verrou ni allocation,
**utilisee par defaut**. Runqueues multi-CPU, affinites, vol de travail.
Preemption depuis l'IRQ (`preempt_from_irq`) et preemption differee
(`request_deferred_preempt`).

*Preuve :* `test_runqueue_ng.rs` (15), `test_latence_centiles.rs` (8),
`test_runqueue_irq.rs` (9).

**Ce qui manque.** La preemption existe depuis l'interruption, pas aux
**points surs** d'un appel systeme long. Aucun timer tickless (`0` occurrence).
Le scheduler n'a pas de notion de charge UI : renderer, compositeur, reseau et
entree ont le meme profil, alors que ce sont eux qui font ou defont la
fluidite percue.

**Ce qui bloque.** Rien d'architectural. Le point dur est le chantier 1 : un
noyau preemptible aux points surs demande que ces points ne tiennent pas le
gros verrou.

## 3 — Memoire NG 🔵

**Ce qui existe.** Le scan O(n) des frames libres est devenu un bitmap O(1).
Un depot de magasins et une arene DMA avec liberation. Un page-cache avec
`reclaim_pages`. 32 usages de shootdown TLB.

*Preuve :* `test_magasin_depot.rs` (10), `test_arene_dma.rs` (14),
`test_frames_libres.rs`.

**Ce que cette session a change.** Un **allocateur compagnon** existe et sert
le DMA. Il remplace `AreneDma`, qui suivait au plus soixante-quatre regions
rendues et **perdait** la memoire au-dela -- pas corrompue, perdue. Ce plafond
etait atteint par la fragmentation, c'est-a-dire par un pilote de stockage
sous charge : la machine finissait par ne plus pouvoir allouer de DMA sans
qu'aucune erreur ne dise pourquoi.

Il apporte aussi ce qui manquait vraiment : l'allocation CONTIGUE d'ordre N.
Sans elle, un pilote qui voulait seize pages contigues ne pouvait pas les
demander, et c'est pour cela que le DMA avait du se faire une arene a part.

*Preuve :* `test_compagnon.rs` (21), `verifie-compagnon.py` (9 regles).

**Ce qui manque, et c'est encore l'essentiel.** `src/kernel/memory/heap.rs` le
dit toujours : *« The old `LockedHeap` remains the proven backing allocator »*.
`NgHeap` l'enveloppe pour six classes de taille jusqu'a 1024 octets ; au-dela,
tout descend dans le verrou global. Il n'y a **ni slab, ni caches de pages par
CPU pour les frames**, pas de politique de working-set, pas d'OOM propre. Le
compagnon ne sert que le DMA : le faire servir le tas noyau et l'allocateur de
frames est la suite.

## 4 — Graphique NG / compositeur ring 3 🔵

**Ce qui existe.** Un contrat de composition et une tranche verticale ring 3
(`userland/services/composited`), construite en CI. `test_composited.rs` (48
preuves), `verifie-protocole-composited.py`.

**Ce qui manque.** `src/gui/window_manager.rs` fait **89 Ko** -- il a GROSSI
depuis le constat des 82 Ko. Le compositeur noyau reste le chemin par defaut ;
la tranche ring 3 est une preuve de faisabilite, pas le produit. Pas de triple
buffering, pas de frame pacing explicite, pas d'acceleration GPU.

**Ce qui bloque.** Le deplacement est massif et touche a tout : c'est le
chantier le plus couteux des douze, et celui qui rendrait le plus.

## 5 — Systeme de fichiers + E/S moderne 🟡

**Ce qui existe.** Commit A/B a deux demi-zones, generation, sommes de
controle, injection de coupure **exhaustive**. Depuis ce lot : la persistance
passe par la couche bloc generique et non plus par la nappe ATA, et le commit
pose une **vraie barriere** avant et apres le superbloc -- avec un aveu
explicite quand le disque n'en offre pas.

Cote materiel : **NVMe est pilote** (files admin et E/S, PRP, vidange), GPT est
lu et ecrit, FAT32 est ecrit et **valide par `fsck.fat` et `mtools`**.

*Preuve :* `test_commit_crash.rs` (13), `test_nvme_decodage.rs` (31),
`test_gpt.rs` (29), `test_fat32.rs` (25), `verifie-fat32-reel.py`,
`verifie-nvme.py`, `verifie-partitionnement.py`.

**Ce qui manque.** Pas d'allocation par extents. Pas de cache d'ecriture
propre. **Aucune E/S asynchrone** (`0` occurrence dans `src/fs/`) : la couche
bloc a la forme d'un achevement differe, et aucun pilote ne le rend. Pas
d'AHCI.

## 6 — Architecture de securite 🟡

**Ce qui existe.** Les mots de passe sont **sales et haches**, le sel vient de
la source d'aleas du noyau. Cinq profils separes, dont `BrowserNetwork` isole
du rendu. `NET_CONNECT`, `no_new_privs` d'office, exec cote appelant. Les
droits d'ecriture persistants sont bornes par sous-arbre canonique.

*Preuve :* `test_bac_a_sable_navigateur.rs` (15), `test_abi_droits.rs` (19),
`verifie-telechargements.py`, `verifie-installation.py`.

**Corrige depuis.** W^X existe : `src/kernel/security/wx.rs`, applique au
chargement ELF (« segment inscriptible et executable refuse (W^X) ») comme dans
`mmap`/`mprotect`, avec sa suite hote `tools/platform/test_wx.rs` et son
garde-fou `tools/verifie-wx.py`. Le paragraphe ci-dessous decrit l'etat
*anterieur* et est conserve pour l'historique.

**Ce qui manquait alors.** **Aucun W^X / NX** : la recherche de `NO_EXECUTE` ne rend
rien. Pas de randomisation d'adresses. La sandbox du plan M14 -- seccomp-like,
`pledge`/`unveil` -- n'existe pas (2 mentions, aucune implementation). Les
capabilities existent en type mais ne gouvernent pas encore tous les appels.

**Ce qui bloque.** W^X demande de reprendre le mappage des segments ELF et la
pagination : c'est faisable et ce n'est pas petit.

## 7 — ABI Bouchaud + IPC natif 🟡

**Ce qui existe.** L'arbre `src/kernel/native/` porte handles, objets,
evenements, IPC, memoire partagee, waitset, readiness, reseau, temps. Les
droits s'attenuent au transfert.

*Preuve :* `test_abi_droits.rs` (19), `verifie-abi-native.py`,
`native-ipc-runtime.yml`.

**Ce qui manque.** Linux reste la **personnalite dominante** : la compat n'est
pas ecrite au-dessus des primitives natives, elle definit encore implicitement
le systeme. Les erreurs ne sont pas versionnees. Les surfaces graphiques ne
sont pas des objets natifs.

## 8 — Ladybird comme vrai produit navigateur 🔵

**Ce qui existe.** WebContent reel, HTTP/HTTPS, chrome complet (onglets,
Ctrl+F, menu contextuel, telechargements, historique, favoris, presse-papiers,
survol de lien), supervision multi-processus avec budget de relance.

*Preuve :* `test_supervision.rs` (13), `test_calques`, `test_degat`,
`test_nom_fichier`, `test_url`, sept garde-fous de chrome, smoke QEMU en CI.

**Ce qui manque.** **Un seul renderer** : le multi-renderer est M13 et n'est
pas fait. **Aucune sandbox** (M14). Aucune campagne WPT. Le chrome vit encore
DANS WebContent, ce qui est la dette centrale du portage et la raison pour
laquelle deux droits d'ecriture persistants ont du etre accordes au rendu.

## 9 — Reseau NG 🔵

**Ce qui existe.** TCP avec file des segments non acquittes, RTO, Karn,
retransmission rapide, echantillonnage du RTT. DNS, TLS, HTTP/HTTPS. Deux
pilotes : e1000 et **rtl8168** -- ce dernier etant celui de la machine de
reference.

*Preuve :* `test_tcp_retransmission.rs` (23 preuves).

**Ce qui manque.** **Aucun IPv6** (`0` occurrence dans `src/net/`). **Aucun
zero-copie**. Pas de backpressure explicite. La readiness asynchrone existe
cote ABI native (9 usages) mais n'irrigue pas la pile.

## 10 — Plateforme materielle de reference 🟡

**C'est le chantier le plus avance des douze**, et le seul qui ait progresse
sur du materiel physique.

**Ce qui existe.** Cible choisie : **TRIGKEY Speed S5**, Ryzen 7 5700U, 32 Go,
NVMe. Amorcage **UEFI** prouve sur la machine, framebuffer GOP natif, ACPI
(HPET, MCFG, MADT, FADT), PCIe avec BAR 64 bits et MSI/MSI-X, **xHCI avec HID
clavier et souris**, **NVMe pilote**, reseau rtl8168, et depuis ce lot une
**installation reelle sur le disque interne** -- table GPT, ESP FAT32 validee
par des outils etrangers, partition systeme persistante.

*Preuve :* `test_pci_decodage.rs` (16), `test_nvme_decodage.rs` (31),
`test_hid.rs` (26), `test_installation.rs` (16), `test_gpt.rs`, `test_fat32.rs`,
`verifie-matrice-materielle.py`, dix garde-fous `tools/reference/`.

Depuis ce lot, l'entree est **complete de bout en bout** : les SMI du
micrologiciel sont desarmes apres la prise du semaphore -- sans quoi le BIOS
continue d'intercepter chaque evenement USB et le clavier « marche dans le
BIOS, pas dans le systeme » --, le repli PS/2 se decide genre par genre sur ce
qui a REPONDU et non sur la presence d'un controleur, et **les concentrateurs
sont traverses** : chaine de route, bit `Hub`, transactionneur, requetes de
classe. Un clavier branche sur un hub repond. La commande `lsusb` rejoue
l'arbre depuis l'etat, indentation comprise.

*Preuve supplementaire :* `test_concentrateur.rs` (35), `verifie-entree-trigkey.py`
(12 mutations attrapees), `verifie-concentrateurs-usb.py` (17 mutations
attrapees), et surtout `tools/ci/run_usb_arbre.sh` -- un clavier ET une souris
branches DERRIERE un concentrateur sous QEMU, rien en direct, dans
`integration-gate`.

**Ce qui manque.** Audio : seulement AC97, pas de HDA. **Pas de Wi-Fi.** Pas de
batterie, pas de temperatures, **pas de suspend/resume**. Pas de GPU. Au-dela
de cinq concentrateurs en cascade la chaine de route xHCI est pleine -- limite
du materiel, nommee dans le journal.

## 11 — Fiabilite / CI / release engineering 🔵

**Ce qui existe.** L'observabilite est la meilleure partie du projet :
instrumentation du BKL, du GUI, du scheduler, des trames, des blocages, et des
tests qui falsifient volontairement les invariants. 67 garde-fous, 58 suites
hote, 18 workflows dont `soak.yml`, `endurance.yml`, `reliability-v3.yml`.
`release.yml` demande des attestations.

**Ce qui manque, et c'est grave.** **`main` n'est pas protege.** L'API GitHub
rend `"protected": false` sur les trois branches. `tools/ci/configure_protection.ps1`
existe et n'a jamais ete applique : il y a un script pour poser la protection,
et aucune protection. **Aucun required status check** : tout ce travail de
garde-fous peut etre contourne par un `git push` direct.

**Le fuzzing couvre desormais les decodeurs d'octets etrangers.**
`tools/fuzz/test_decodeurs_fuzz.rs` (18 proprietes) rejoue sur 64 graines les
decodeurs qui lisent ce que le noyau ne controle pas : table de partitions
GPT, secteur d'amorcage FAT32, `Identify` NVMe, rapports HID, et des suites
d'operations quelconques sur l'allocateur compagnon.

C'est la difference entre un defaut et une surface d'attaque : brancher une
cle USB fabriquee ne demande aucun privilege. Le fuzzing a immediatement
trouve un cas reel -- `fat32::ouvre` acceptait un volume a ZERO amas, et le
premier acces calculait alors un secteur que personne n'avait choisi.

Le job de CI ne connaissait qu'une suite ; il les DECOUVRE maintenant. Une
suite de fuzzing qui n'est pas lancee ne protege rien tout en donnant
l'impression du contraire.

Ce qui reste sans fuzzing : le decodage des paquets reseau, les descripteurs
USB de configuration, et les entrees du systeme de fichiers persistant.

Les budgets d'execution (`ready_latency_*`, `tcp_busy_poll_tours_max`) sont
rapportes « non verifies » faute de campagne QEMU, ce qui est le comportement
voulu de `check_budgets.py` et la seule lecture honnete.

**Le remede le moins cher du projet, et il est pret.** Les trois verdicts
`fast-gate`, `integration-gate` et `reliability-gate` existent deja, portent
`if: always()` -- donc rapportent meme quand le reste est saute -- et se
declenchent sur `pull_request`. Il ne manque que la commande qui les rend
obligatoires.

`tools/ci/configure-protection.sh` la porte, sans dependre de PowerShell.
`tools/verifie-protection-main.py` verifie que les noms exiges correspondent
toujours a des verdicts qui existent : une protection qui exige un check
disparu ne casse pas bruyamment, elle bloque TOUTES les PR pour toujours, et
le remede evident est alors de la retirer en entier.

## 12 — Polish produit ⚪

**Ce qui existe.** L'echelle fractionnaire et les coordonnees logiques sont
dans le protocole (`test_protocole.rs`, 25 preuves), retrocompatibles. Le
presse-papiers de bureau existe. Les animations de fenetre existent.

**Ce qui manque.** La recherche rend **zero** occurrence pour HiDPI, **zero**
pour glisser-deposer. L'accessibilite et l'IME n'existent pas. Pas de
notifications, pas de reglages, **pas de mise a jour atomique ni de recovery**.
Les modes Work/Focus/Gaming ne sont pas ecrits.

C'est le seul chantier qu'on peut honnetement declarer **non commence**, et
c'est normal : il se pose sur les onze autres.

---

## Ce que cet audit ne dit pas

Il ne dit pas combien de temps chaque chantier demande. Il ne dit pas non plus
qu'aucun n'est termine par accident : **aucun des douze n'est termine**, et
deux d'entre eux (4 et 12) n'ont pas de chemin par defaut du tout.

Il dit ou se trouve le levier. Par ordre de rapport entre cout et effet :

1. **Proteger `main`** (chantier 11). Une commande --
   `tools/ci/configure-protection.sh` --, et elle rend executoires les
   soixante-huit regles ecrites. Elle demande un jeton portant
   `administration:write`, que l'environnement d'integration n'a pas : c'est
   au proprietaire du depot de la lancer une fois.
2. **Generaliser la memoire NG** (chantier 3). Le compagnon existe et ne sert
   que le DMA ; `LockedHeap` reste le fond du tas noyau au-dela de 1024
   octets, et c'est lui qu'on paie a chaque grosse allocation.
3. **Finir la sortie du BKL** (chantier 1). Les sockets, `futex`, `openat` et
   `ioctl` ; c'est ce qui debloque la preemption aux points surs (chantier 2).
4. **Sortir le compositeur** (chantier 4). Le plus cher, et celui qui produit
   le « 60/120 Hz feel ».
