# P10 — GPU : audit, verdict BLOCKED, premiers composants

> État au 2026-10-08, HEAD `c63a328e` (section 4 bis : 2026-10-14, `05b542bf`), Ladybird épinglé
> `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6`. Niveau de preuve de chaque
> constat entre crochets : **[STATIQUE]** (lu dans le code), **[QEMU-TCG]**
> (vu sur un noyau qui tourne).

## Verdict

**P10 = BLOCKED.** Bouchaud OS n'a aucun backend GPU ; Ladybird y peint en
CPU (Skia raster, `--force-cpu-painting`) et le gestionnaire de fenêtres
présente par copie dans un framebuffer linéaire (BGA sous QEMU, GOP sur la
Trigkey). Aucun faux backend n'est installé pour faire passer P10 en vert :
un Vulkan logiciel (lavapipe) dans l'invité exercerait le code GPU de
Ladybird sans rien accélérer, et serait plus lent que le raster Skia
d'aujourd'hui.

Le premier composant à écrire est **le transport virtio-PCI moderne du noyau**
(section 4). Sans lui, aucun périphérique virtio n'existe pour Bouchaud,
qu'il s'agisse du GPU, du réseau ou du disque.

## 1. Ce qui existe

| Pièce | État | Preuve |
|---|---|---|
| Contrat GPU du noyau (`src/drivers/api/gpu.rs`) | un seul backend : `BgaLinearFramebuffer` ; virtio-gpu cité comme « pourra » | [STATIQUE] |
| Affichage QEMU | BGA/Bochs (`src/drivers/display/bochs.rs`), 1280×720, LFB | [QEMU-TCG] toute la CI |
| Affichage Trigkey | GOP UEFI 1920×1080/32 bpp (`src/platform/pc/reference_gop.rs`) | physique, docs Trigkey |
| Énumération PCI | parcours derrière les ponts, capacités (`capacites_de`), BAR 64 bits (`bar_decode`), MSI-X déjà programmé par RTL8168 et xHCI | [STATIQUE] |
| virtio | **aucun** pilote ; `pci.rs` ne connaît que le nom du vendeur `0x1AF4` | [STATIQUE] |
| `-device virtio-gpu-pci` sous QEMU | au 2026-10-08 : énuméré, aucun pilote. Depuis : transport (section 4) puis scanout 2D (section 4 bis) ; avec BGA, le bureau reste sur BGA | [QEMU-TCG] |
| Ladybird, Compositor | `initialize_gpu_backend()` n'est appelé que sans `--force-cpu-painting` ; sous Bouchaud l'option est toujours passée | [STATIQUE] `Services/Compositor/main.cpp` |
| Ladybird, build | `browser-vcpkg.sh` retire `vulkan`, `vulkan-headers` et `vulkan-memory-allocator` du manifeste, et la construction échoue s'ils y réapparaissent | [STATIQUE] |

## 2. Ce que Ladybird demande pour peindre sur GPU

Sur Linux, l'arbre épinglé n'a **qu'un** chemin GPU : Vulkan.

- `Meta/CMake/check_for_dependencies.cmake` définit `USE_VULKAN` si
  `VulkanHeaders` et `Vulkan` sont trouvés. Il définit aussi
  `USE_VULKAN_DMABUF_IMAGES` sur Linux quand `glslangValidator` est présent.
- `SkiaBackendContext::create_independent_gpu_backend()` choisit Metal (macOS),
  Direct3D 12 (Windows) ou Vulkan, et rien d'autre. Il n'y a pas de backend
  OpenGL pour la peinture.
- WebGL est un chemin séparé : ANGLE (`libEGL`, `libGLESv2`) côté WebContent.
  Le build Bouchaud ne le contient pas.
- `USE_VULKAN_DMABUF_IMAGES` partage les images du Compositor avec l'UI sous
  forme de dma-buf. Sans import dma-buf dans le gestionnaire de fenêtres,
  chaque cadre devrait être relu du GPU vers la RAM avant d'être présenté.

## 3. Les deux cibles, et pourquoi aucune n'est atteignable dans ce lot

### QEMU : virtio-gpu + Venus (Vulkan sérialisé vers l'hôte)

Chaîne complète, de bas en haut :

1. **Transport virtio-PCI moderne** (noyau) : capacités vendeur
   `common/notify/isr/device`, files *split* (descripteurs, *available*,
   *used*), négociation des fonctionnalités, MSI-X.
2. **virtio-gpu 2D** (noyau) : `GET_DISPLAY_INFO`, `RESOURCE_CREATE_2D`,
   `ATTACH_BACKING`, `SET_SCANOUT`, `TRANSFER_TO_HOST_2D`, `RESOURCE_FLUSH`.
   C'est un remplaçant de BGA. Il **n'accélère rien**.
3. **virtio-gpu 3D** (noyau) : `VIRTIO_GPU_F_VIRGL`, `F_RESOURCE_BLOB`,
   `F_CONTEXT_INIT`, capsets, ressources blob, mappage du BAR *hostmem*,
   `SUBMIT_3D`, clôtures (*fences*).
4. **uAPI DRM compatible Linux** : `/dev/dri/renderD128`, plus les ioctls
   `DRM_IOCTL_VIRTGPU_*` (`GETPARAM`, `GET_CAPS`, `CONTEXT_INIT`,
   `RESOURCE_CREATE_BLOB`, `MAP`, `EXECBUFFER`, `WAIT`). S'y ajoutent les
   ioctls DRM génériques (GEM close, PRIME fd↔handle, syncobj), les fd
   dma-buf, les *sync files* et le sysfs que libdrm parcourt.
5. **Mesa Venus** (pilote Vulkan invité) et libdrm, construits pour la couche
   Linux de Bouchaud.
6. **Ladybird** reconstruit avec `USE_VULKAN`. Le Compositor est alors lancé
   sans `--force-cpu-painting`, et le gestionnaire de fenêtres doit importer
   le dma-buf, ou accepter une relecture par cadre.
7. **Hôte** : QEMU avec `virtio-gpu-gl,venus=on,blob=on,hostmem=…`,
   virglrenderer compilé avec Venus, et un pilote Vulkan hôte.

Bloquant de banc : les exécutants GitHub n'ont pas de GPU. Venus y tournerait
sur lavapipe côté hôte. Cela prouverait que la chaîne fonctionne, mais ne
dirait rien d'une accélération : la mesure resterait un rendu CPU,
simplement déplacé hors de la VM.

### Trigkey : AMD Ryzen 7 5800H, iGPU Radeon Vega 8 (Cezanne)

La seule voie réelle est un pilote `amdgpu` :

- affichage DCN 2.1 ;
- anneaux GFX9 et SDMA ;
- chargement des microcodes par le PSP ;
- SMU ;
- gestion de la VRAM partagée et de la GART ;

le tout surmonté de RADV (Mesa) sur la même uAPI DRM que ci-dessus. C'est
d'un autre ordre de grandeur que tout le reste de P1–P13. Sur la Trigkey,
l'affichage reste le GOP.

## 4. Premier composant, et sa définition de DONE

**Transport virtio-PCI moderne** (`src/drivers/virtio/`, à créer). Pourquoi
celui-là :

- toutes les étapes 2 à 7 en dépendent ;
- il sert aussi au réseau et au disque virtio, qui seraient plus rapides que
  e1000 et ATA sous QEMU ;
- l'énumération PCI, le décodage des BAR 64 bits et MSI-X qu'il exige
  existent déjà.

DONE quand :

1. le noyau négocie `VIRTIO_F_VERSION_1` avec un périphérique de QEMU ;
2. il fait un aller-retour sur une file (pour virtio-gpu,
   `GET_DISPLAY_INFO`, qui rend la taille de l'écran) ;
3. le journal série le publie ;
4. un banc QEMU l'exige, et un garde statique protège le marqueur.

**État : premier composant FAIT sous QEMU-TCG** (`BOUCHAUD_VIRTIO_PCI_V1`,
`src/drivers/display/virtio_gpu.rs`, banc `tools/ci/run_virtio_gpu.sh`, garde
`tools/verifie-virtio-pci.py`). Relevé local du 2026-10-08 :

    VIRTIO_PCI peripherique=1af4:1050 bus=00:04.0 commune=1 notification=1 isr=1 config=1
    VIRTIO_GPU_INIT fonctionnalites_proposees=0x0000010130000002 retenues=0x0000000100000000 files=2 controlq=64/64
    VIRTIO_GPU_DISPLAY_INFO ecrans_actifs=1 ecran0=1280x800 delai_us=224
    BOUCHAUD_VIRTIO_GPU_OK ecran0=1280x800

Sans `-device virtio-gpu-pci` : une ligne `VIRTIO_GPU absent`, rien d'autre.
L'aller-retour est scruté (pas d'interruption) et borné à 1 s. La preuve
QEMU-KVM vient de l'étape os-primitives.

## 4 bis. Deuxième composant : scanout virtio-gpu 2D (sans accélération)

`BOUCHAUD_VIRTIO_GPU_2D_V1`, au-dessus du transport (inchangé). Il **remplace
BGA** quand BGA manque. Il n'accélère rien : le bureau et Ladybird restent
peints par le processeur, aucune commande 3D, aucun VIRGL, aucun contexte.
**P10 reste BLOCKED.**

| Pièce | Où | Preuve |
|---|---|---|
| Commandes 2D synchrones : `RESOURCE_CREATE_2D`, `ATTACH_BACKING` (DMA contiguë), `SET_SCANOUT`, `TRANSFER_TO_HOST_2D`, `RESOURCE_FLUSH`, `DETACH_BACKING`, `RESOURCE_UNREF` | `virtio_gpu.rs` | [QEMU-TCG] |
| Autotest au démarrage : chemin positif complet, **7 refus** chacun avec son code (format 0, ressource 0, doublon, transfert et flush sur ressource inconnue, scanout 16, rectangle hors limites) | `virtio_gpu.rs` | [QEMU-TCG] `VIRTIO_GPU_2D negatifs refuses_avec_le_bon_code=7` |
| Mire 64×64 à l'écran virtio, **exacte au pixel** (capture `screendump` du moniteur QEMU) | banc | [QEMU-TCG] `SCANOUT_MIRE_OK pixels_exacts=3721/3721` |
| Politique : GOP > BGA > virtio-gpu 2D (si autotest vert) > aucun | `politique_scanout.rs` | [HÔTE] 5 tests ; [QEMU-TCG] |
| Bureau présenté par virtio-gpu sans BGA (`-vga none`) : chaque `present`/`present_rect` devient transfert + flush du rectangle | `bochs.rs` | [QEMU-TCG] capture 1280×720 identique au bureau BGA, 921 404/921 600 pixels (seule l'horloge diffère) |
| Présentation continue | banc | [QEMU-TCG] deux captures à 3 s : l'horloge a changé à l'écran (`SCANOUT_VIVANT_OK`) |
| Repli : ni BGA ni virtio → `present() inactif (scanout=aucun)`, pas de panique ; BGA présent → le bureau reste sur BGA | banc | [QEMU-TCG] [QEMU-KVM] |
| Commande sans réponse : file cassée, scanout abandonné à la première présentation ; mémoire DMA rendue seulement si l'état est sûr (§4 ter) | `virtio_gpu_dma.rs` | [HÔTE] 7 tests ; [QEMU-TCG] banc de panne injectée |
| `/dev/fb0` : `mmap` et cession au ring 3 **refusés** sur ce scanout (un client n'y a aucun moyen de présenter) | `bochs.rs` | [STATIQUE] |
| Mesure | `[GUI-VIRTIO-GPU]` | [QEMU-TCG] 25 présentations en 30 s, 0 échec, 449 µs par présentation en local, 228 µs en CI ; [QEMU-KVM] 182 µs (bureau au repos : 1 plein écran + l'horloge) |

Banc : `tools/ci/run_virtio_gpu_scanout.sh` (os-primitives, TCG puis KVM).
Contre le faux vert : chaque démarrage doit atteindre son point de capture,
une capture absente est un échec, un aplat ne vaut pas une image, et une
mutation (`presente()` sans transfert ni flush) rend le banc **rouge**
(0/921 600 pixels, horloge figée).

**Preuve CI, os-primitives `585fc0a0` (run 38530194699, job 115739969599)** :

| | TCG | KVM |
|---|---|---|
| mire | 3721/3721 | 3721/3721 |
| bureau virtio = bureau BGA | 921 313/921 600 | 921 130/921 600 |
| horloge vivante (pixels changés) | 135 (76 horloge) | 83 (83 horloge) |
| présentations / échecs | 26 / 0 | 28 / 0 |
| coût d'une présentation | 228 µs | **182 µs** |

**État : scanout virtio-gpu 2D VERIFIED_AT_KVM (QEMU).** P10 reste BLOCKED :
aucune accélération, aucune 3D, aucun GPU AMD.

Ce qui manque encore pour dire « le bureau sur virtio-gpu » au-delà :

- ~~Ladybird sous `-vga none -device virtio-gpu-pci`~~ : **montré** par le
  travail `browser-host smoke sur virtio-gpu` (`a0e7bdd3`, run 38536498356,
  job 115761496106, TCG) : scanout virtio actif, 204 présentations, 0 échec,
  38,4 Mpixels transférés (861 µs par présentation), mire du navigateur
  vérifiée sur l'écran virtio (`surface=1`), `LADYBIRD_CONVERGENCE_OK`. Ce
  travail est diagnostic (hors du verdict de convergence) ;
- un client ring 3 qui peint `/dev/fb0` directement (refusé, voir plus haut) ;
- les interruptions : les commandes sont scrutées, bornées à 1 s ;
- et rien de tout cela ne concerne la Trigkey (Radeon Vega, pas virtio).

## 4 ter. Mémoire DMA après une commande sans réponse

`BOUCHAUD_VIRTIO_GPU_DMA_ETAT_V1` (revue du 14 octobre, priorité haute).

**Le défaut.** `active_scanout` rendait sa mémoire DMA si elle n'avait
« jamais été attachée », un booléen posé seulement quand `ATTACH_BACKING`
répondait OK. Si l'attente bornée de la réponse échouait, le booléen restait
faux. Pourtant la requête était partie et le périphérique avait pu retenir
l'adresse. La mémoire était alors rendue à l'allocateur et réutilisée, alors
que le périphérique pouvait encore y lire (transfert) ou y écrire. Le même
raisonnement valait pour la mire de l'autotest.

**Le modèle** (`src/drivers/display/virtio_gpu_dma.rs`, module pur). Chaque
tampon a un état : `NonAttachee`, `Attachee` ou `Inconnue`. Chaque commande a
une issue :

- `Acceptee` ;
- `Refusee` (le périphérique a répondu par une erreur) ;
- `SansReponse` (la requête est partie, la réponse n'est pas venue) ;
- `NonEnvoyee` (file déjà cassée, requête trop longue).

Les transitions sont les suivantes :

- un `ATTACH_BACKING` ou un `DETACH_BACKING` sans réponse rend l'état
  `Inconnue` ;
- une autre commande sans réponse sur une mémoire attachée la rend aussi
  `Inconnue` (un transfert peut encore la lire) ;
- `Inconnue` est **absorbant** ;
- un détachement refusé ne libère rien.

`free_dma` n'existe qu'à un endroit, `rend_si_sur`, derrière
`dma::peut_rendre` (`NonAttachee` seulement). Sinon la mémoire est
**conservée** et le journal le dit : `VIRTIO_GPU_DMA memoire_conservee quoi=…
octets=… etat=…`. Une fuite bornée (3,6 Mo au pire, une fois, la file étant
cassée ensuite) vaut mieux qu'une corruption.

| Preuve | Niveau |
|---|---|
| `tools/display/test_virtio_gpu_dma.rs` : 7 tests, dont le scénario du défaut. Le modèle de l'ancienne décision (`!attachee \|\| liberee`) y rendait la mémoire ; le nouveau la garde. | [HÔTE] |
| **Banc de panne injectée** `tools/ci/run_virtio_gpu_dma_panne.sh`. Le noyau est compilé avec l'option `banc-virtio-gpu-sans-reponse` (jamais dans `default`, garde 8). La requête `ATTACH_BACKING` du bureau **part vers QEMU**, qui la reçoit et retient l'adresse, puis l'attente est déclarée échue. Exigé : scanout refusé à l'attache, `memoire_conservee quoi=bureau octets=3686400 etat=Inconnue`, bureau replié (`present() inactif`), aucune panique. | [QEMU-TCG] local ; CI os-primitives TCG + KVM |
| Mutation : l'attache sans réponse laisse l'état inchangé (l'ancien comportement). Le banc de panne passe **rouge** (`la memoire du bureau est CONSERVEE` → ECHEC). | [QEMU-TCG] local |
| Garde `verifie-virtio-pci.py`, règles 6 et 8 : un seul `free_dma`, derrière `peut_rendre` ; l'issue de l'attache suivie (scanout et mire) ; l'injection confinée à l'option et passée en CI. 5 mutations rouges (8 pour toute la garde). | [STATIQUE] |

## 4 quater. Verrou du périphérique pendant une commande synchrone

`BOUCHAUD_VIRTIO_GPU_VERROU_DORMANT_V1` (revue du 14 octobre).

**Audit de l'ancien code.** `static GPU: SpinLock<Option<Peripherique>>` était
tenu pendant toute la commande, réponse comprise. L'attente était bornée à
1 s.

- **Ordonnanceur.** Un `SpinLock` marque son cœur non préemptible
  (`verrous_simples`) : le tic refuse d'y commuter. Chaque présentation
  immobilisait donc son cœur pendant ses deux commandes, 182 µs (KVM) à
  861 µs (TCG, Ladybird). Avec un périphérique muet, l'immobilisation durait
  **1 s par commande**. En SMP1, c'était tout le système qui se figeait. Un
  second demandeur sur un autre cœur tournait lui aussi, non préemptible :
  deux cœurs perdus.
- **Interruptions.** Elles ne sont pas masquées (`SpinLock` ne fait pas
  `cli`). Les gestionnaires passaient, mais tout ce qui demande une élection
  (réveils, préemption ciblée) attendait la fin de la commande sur ce cœur.
- **Fluidité.** Au rythme du bureau (≤ 60 présentations/s), 1 à 5 % d'un cœur
  étaient non préemptibles. C'était sans effet visible au repos, mais une
  latence d'élection s'ajoutait à chaque présentation pendant une rafale.

**Correctif.**

- `SleepMutex` (le demandeur concurrent dort au lieu de tourner), pris par
  `verrou()`, et seulement ainsi.
- `lock()` n'est utilisé que si céder est sûr. La condition est
  `preempt::cession_volontaire_sure()`, nouvelle : les interdits de la
  préemption d'un fil noyau (aucun verrou tournant tenu, ni `preempt_count`,
  ni lecture du registre, ni shootdown, ni sortie en cours), mais IF ouvert et
  hors IRQ.
- Ailleurs, c'est `try_lock()` : la sonde au démarrage (sans tâche) passe ;
  une présentation qui ne peut pas prendre le verrou sans bloquer est
  **sautée et comptée**.
- L'attente de réponse (`attente_reponse`) tourne 50 µs, puis **cède** le
  cœur entre deux lectures si le contexte le permet. Sa borne datée (1 s) et
  son plafond TSC sont inchangés.

La ligne `[GUI-VIRTIO-GPU]` porte désormais `sautes=`, `cedes=` et
`non_cedables=`. Une attente non cédable **dans une tâche** signifierait
qu'un appelant tient un verrou tournant : elle est journalisée
(`VIRTIO_GPU_ATTENTE non_cedable_en_tache`, avec ce qui l'interdit) et le
banc l'exige absente.

| Mesure locale [QEMU-TCG], bureau au repos, 30 s | Avant (SpinLock) | Après |
|---|---|---|
| présentations / échecs | 25 / 0 | 24 / 0 |
| coût d'une présentation | 449–531 µs | 399–530 µs (3 démarrages) |
| présentations sautées | — | 0 |
| attentes non cédables dans une tâche | non mesuré | **0** (les 13 non cédables sont la sonde et l'autotest, hors tâche) |

Le diagnostic temporaire de chaque refus a montré `spin=0` partout : aucune
présentation n'est faite sous un verrou tournant. La première version du
correctif (IF + tâche seulement) aurait donc été sûre **ici**, mais pas par
construction. La condition complète le garantit.

Garde 7 : `SleepMutex`, un seul `GPU.lock()` dans `verrou()` avec repli
`try_lock`, céder seulement par `cession_volontaire_sure`. 3 mutations rouges
(retour au `SpinLock`, borne datée retirée, prédicat affaibli).

## 4 quinquies. BGA contre virtio-gpu 2D, même charge ; groupement retiré

`BOUCHAUD_BANC_PRESENTATION_V1`. Le bureau au repos ne présente presque rien
(l'horloge, une fois par seconde) : comparer les deux écrans sur lui revient à
comparer deux silences.

La commande `banc-present` présente la **même** suite de trames sur l'écran
actif, par catégorie :

- plein écran 1280×720, 40 fois ;
- bande 1280×30, 200 fois ;
- tuile 64×64, 400 fois.

Elle chronomètre chaque `present_rect` :

- la copie vers l'écran ;
- sous virtio-gpu, `TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH` et leurs réponses.

Le dessin dans le tampon arrière n'est pas compté, car il est identique pour
les deux écrans. Banc `tools/ci/run_banc_presentation.sh` : deux démarrages
(`-vga std` ; `-vga none -device virtio-gpu-pci`), en CI os-primitives sous
TCG puis KVM. Exigé : les trois catégories sur le bon écran ; sous virtio,
640 présentations acceptées, aucun échec, scanout actif à la fin. Une
`presente()` vidée de ses commandes rend la dernière exigence rouge.

**Mesure locale [QEMU-TCG], avant regroupement** (`e788a77b`, moyenne / p95,
en µs) :

| Catégorie | BGA | virtio-gpu 2D | Rapport |
|---|---|---|---|
| plein 1280×720 | 1 892 / 2 729 | 3 506 / 7 161 | 1,9× |
| bande 1280×30 | 74 / 106 | 179 / 318 | 2,4× |
| tuile 64×64 | 23 / 36 | 141 / 197 | 6,1× |

Lecture :

- **coût fixe** : environ 115 µs par présentation pour les deux commandes
  (deux sorties de la machine virtuelle, deux attentes de réponse), visibles
  surtout sur les petits rectangles ;
- **coût proportionnel** : la copie vers la mémoire DMA, puis la copie de
  l'hôte vers sa surface (`TRANSFER_TO_HOST_2D`).

**Latence de trame.** Sous virtio-gpu, la réponse au `FLUSH` — barrière
comprise, depuis BARRIERE_V1 ; sans elle, la spécification ne garantit rien —
dit que l'hôte a mis à jour sa surface ; elle est donc incluse dans le coût
mesuré. Sous BGA,
rien ne l'indique à l'invité : l'affichage de QEMU relit la VRAM à son propre
rythme. La seule latence de trame observable par l'invité est donc celle de
virtio-gpu.

**Groupement `BOUCHAUD_VIRTIO_GPU_LOT_V1` (`01f3063a`) : retiré.** Il
envoyait le transfert et le flush sous **une** sonnette, avec deux chaînes,
puis attendait `dernier_util + 2`. Sa justification, « la file de contrôle
est traitée dans l'ordre », est **fausse** au regard de la spécification
(`virtio-spec`, `device-types/gpu/description.tex`) :

> The device MAY process controlq commands asynchronously and return them
> to the driver before the processing is complete. If the driver needs to
> know when the processing is finished it can set the
> VIRTIO_GPU_FLAG_FENCE flag in the request. The device MUST finish the
> processing before returning the command then.

Rien n'y garantit un ordre de traitement. `VIRTIO_F_IN_ORDER` (bit 35) ne
porte que sur l'ordre d'**utilisation** des tampons, et QEMU ne l'offre pas :
`fonctionnalites_proposees=0x0000010130000002`. Le lot ne contrôlait pas non
plus les identifiants des entrées utilisées. Un flush traité avant son
transfert affiche un rectangle pas encore reçu. Un `DETACH_BACKING` rendu
avant la fin de son traitement ne prouve pas que la mémoire est lâchée.

**Remplacement : `BOUCHAUD_VIRTIO_GPU_BARRIERE_V1`**
(`src/drivers/display/virtio_gpu_file.rs`, module pur) :

- **toute** commande part avec `VIRTIO_GPU_FLAG_FENCE` et un `fence_id`
  croissant, posés **avant** la sonnette ; sa réponse prouve que le
  traitement est fini ;
- **une seule** commande en vol (chaîne de tête 0), une seule sonnette dans
  le pilote ; le flush ne part qu'après la réponse du transfert ;
- chaque réponse est **jugée** (`file::juge`) :
  - exactement une entrée utilisée de plus ;
  - identifiant égal à la tête de chaîne ;
  - longueur d'au moins un en-tête, et au plus la capacité du descripteur
    écrivable ;
  - drapeau `FENCE` renvoyé, avec le **même** `fence_id`.
- l'attente se termine sur **tout** progrès de `used.idx`, toujours bornée
  (1 s datée + plafond TSC). Une avance de deux entrées est une violation,
  pas une réponse. `dernier_util` n'avance qu'après jugement ;
- une violation (`VIRTIO_GPU_FILE violation=… commande=… barriere=…
  attendue=…`) casse la file et rend `REPONSE_INVALIDE`. Elle pèse comme
  `SANS_REPONSE` sur l'état DMA : mémoire `Inconnue`, jamais rendue.

Ce que le jugement attrape, testé sur l'hôte
(`tools/display/test_virtio_gpu_file.rs`, 12 tests) :

- achèvement désordonné (id d'une autre chaîne) ;
- réponse rendue avant la fin du traitement (sans barrière) ;
- réponse tardive d'une commande précédente (barrière n − 1) ;
- deux entrées d'un coup, ou aucune ;
- réponse trop courte, ou plus longue que le descripteur ;
- réponse laissée à zéro ;
- rebouclage de `used.idx`.

Côté invité, le banc de panne `tools/ci/run_virtio_gpu_reponse_etrangere.sh`
utilise l'option `banc-virtio-gpu-reponse-etrangere`, hors `default`. Le
premier transfert du bureau y est jugé contre la barrière d'une **autre**
commande. Exigé :

- la violation est nommée et chiffrée (`barriere_recue = attendue − 1`) ;
- la file est cassée et le scanout abandonné **au premier échec** ;
- aucune commande ne part sur la file cassée ;
- le compositeur vit encore ;
- aucune panique.

Local [QEMU-TCG] : vert, `barriere=19 attendue=20`. En CI os-primitives :
TCG puis KVM. Garde `verifie-virtio-pci` règle 9 : 21 mutations, toutes
rouges (barrière absente ou posée après la sonnette, réponse non jugée,
violation prise pour une réussite, file non cassée, indice consommé avant
le jugement, seconde sonnette, groupement réintroduit, flush sans attendre
le transfert, attente sur l'indice exact, injection hors option).

**Le coût de la sûreté, mesuré, pas caché.** A/B local [QEMU-TCG] sur la
même machine, 3 tours alternés : `80ef1e76` (LOT_V1) contre l'arbre
BARRIERE_V1, moyenne virtio en µs :

| Catégorie | LOT_V1 (3 tours) | BARRIERE_V1 (3 tours) | Écart des moyennes |
|---|---|---|---|
| tuile 64×64 | 75 / 78 / 102 | 100 / 120 / 132 | +38 % (85 → 117) |
| bande 1280×30 | 146 / 137 / 106 | 129 / 227 / 143 | +28 % (130 → 166), bruité |
| plein écran | 1 701 / 1 821 / 2 106 | 1 766 / 1 845 / 2 127 | +2 % |

Le second aller-retour coûte environ 30 µs sur les petits rectangles, et
rien de mesurable en plein écran. QEMU traitant ses commandes 2D de façon
synchrone, la barrière elle-même ne coûte rien. Ce gain-là ne vaut pas une
garantie d'ordre que la spécification ne donne pas.

Pour mémoire, mesure CI de `e9f94b3e` **avec** LOT_V1, en µs (BGA / virtio) :

| Catégorie | KVM | TCG |
|---|---|---|
| plein écran | 195 / 181 | 834 / 1 004 |
| bande | 1 / 20 | 27 / 64 |
| tuile | 1 / 20 | 7 / 30 |

La mesure KVM de BARRIERE_V1 viendra du run os-primitives qui suit ce
correctif.

Pixels inchangés, local [QEMU-TCG] :

- mire 3721/3721 ;
- bureau virtio = BGA, 921 391/921 600 pixels ;
- horloge vivante ;
- 26 présentations, 0 échec, 0 sautée.

Scanout 2D seulement : aucune accélération, aucune 3D, aucun Vulkan. Ce qui
reste à consolider avant toute 3D :

- une file d'interruptions au lieu du scrutin ;
- un tampon de présentation double, pour ne plus copier sous le verrou ;
- la mesure KVM de BARRIERE_V1 (CI, au SHA de ce correctif).

## 5. Ce que P10 ne prétend pas

- Il n'y a ni WebGL, ni WebGPU, ni canvas accéléré. Le canvas 2D est peint en
  CPU par Skia, dans le Compositor.
- Aucune mesure n'a comparé le raster CPU à un GPU. Le débit d'affichage
  mesuré par l'endurance (latence d'un cadre p50/p95/p99) est celui du
  raster CPU.
- `virtio-gpu (QEMU) ≠ GPU de la Trigkey` : une preuve sous QEMU ne
  vaudrait jamais pour la machine physique.
