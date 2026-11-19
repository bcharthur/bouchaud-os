# P10 : scanout Stage 2 et textures GPU en anneau 3

Base 12c69eac. Les essais Linux émulent le CPU avec QEMU 10.0.13 TCG.
Le rendu OpenGL est effectué par virglrenderer avec Mesa logiciel,
sous Xvfb. La VM n’expose ni /dev/kvm ni /dev/dri. La réussite d’un BLIT
VIRGL démontre le chemin de commandes et le résultat, indépendamment du
matériel qui exécute le rendu côté hôte.

## Windows 11 et WHPX

~~~powershell
qemu-system-x86_64.exe -device help | Select-String virtio-gpu-gl-pci
qemu-system-x86_64.exe -display help
.\tools\reference\run-reference-stage2-ladybird.ps1 -Virgl -Accel whpx -Smp 1 -Display gtk -SkipPrepare
~~~

Le profil construit une image Stage 2 séparée avec p10-virgl-experimental.
Il conserve le VGA primaire pour le GOP et ajoute un GPU VirtIO secondaire.
Le bureau choisit son scanout après étalonnage VIRGL au pixel. Sélectionner
le second affichage de QEMU pour voir le bureau VirtIO. En cas de perte du
scanout, regarder l’affichage primaire. Le profil sans option -Virgl sert
de bras CPU. Les disques s’ouvrent en snapshot.

Une liste de périphériques sans virtio-gpu-gl-pci arrête le profil avant
le build. Un build qui annonce le périphérique peut encore refuser le
contexte GL au lancement. La combinaison OVMF + VGA primaire + VirtIO
secondaire a démarré sous GTK/GL sur Linux TCG. Le premier essai SDL/GL
a échoué côté QEMU avec « Unable to create OpenGL context >= 3.0 ».
La disponibilité GL sous Windows et sous WHPX reste à mesurer.

Chercher P10_GPU_CAPSET, P10_GPU_CAPSET_LU et
BOUCHAUD_GPU_COMPOSITION_AUTOTEST_OK, puis UEFI scanout=virtio et les
relevés GPU-COMPOSITION et GPU-TRAMES. Ces derniers indiquent surfaces,
trames, pixels transférés, copies CPU et quantiles de présentation.
Le profil journalise RENDERER_HOST=non-verifie. Pour qualifier la RTX
4070, relever le renderer du processus QEMU et vérifier son activité
GPU pendant la charge via les outils Windows/NVIDIA. Une RTX détectée
dans l’hôte ou un périphérique VirtIO négocié ne suffit pas.

La campagne est reproductible avec les deux profils Stage 2 déjà
construits et le même disque Ladybird. Dans PowerShell :

~~~powershell
.\tools\reference\build-reference-stage2.ps1
.\tools\reference\build-reference-stage2.ps1 -Virgl
.\tools\reference\run-stage2-gpu-ab.ps1 -Url https://example.com/ -Sortie .\target\gpu-ab -Accel whpx -Smp 1
~~~

Le banc alterne l’ordre CPU/VIRGL et vérifie les empreintes des deux
images Stage 2 et de Ladybird entre les essais. Les interactions sont
affichées pour être répétées à l’identique. Chaque rapport JSON
référence la série, les diagnostics QEMU et le fichier de mesures,
et distingue un contexte GL refusé d’un périphérique 3D refusé.
Il s’arrête sur un échec et conserve les rapports déjà produits.

Pour qualifier la session, conserver le même exécutable QEMU, la même
image Ladybird, un instantané disque par lancement, la même résolution,
le même nombre de vCPU et la même séquence d’actions. Alterner les
profils avec et sans `-Virgl`, trois essais minimum par profil.
Enregistrer la série et le renderer hôte pour chaque essai. Ouvrir
Ladybird, charger la même URL, fermer puis rouvrir la fenêtre, répéter
l’action, et éteindre l’invité. Comparer les trames présentées, le P95
de présentation et le temps de dessin. Vérifier séparément le repli
vers le GOP primaire lors d’une perte du périphérique, avec capture
des deux affichages avant et après le repli.

Sur un hôte Linux avec accès à KVM et à un GPU, laisser Mesa sélectionner
le pilote matériel et vérifier le renderer GL du processus QEMU.
Lancer le banc avec `BO_QEMU_KVM=1` et sans `LIBGL_ALWAYS_SOFTWARE`.
Si QEMU WHPX ne peut initialiser le contexte GL, conserver la commande,
la version et le message de refus avec les résultats Linux.

## Interface ring 3

/dev/dri/renderD128 est synthétique et disponible après lecture d’un
capset VIRGL. GETPARAM et GET_CAPS renvoient des données tirées du
périphérique, bornées par le tampon de réponse, sans DMA vers
l’application. Le nœud indique 226:128 à stat/fstat. Le masque annonce
uniquement le capset actuellement consultable. Les fonctionnalités
blob, mappage et context-init ne sont pas annoncées.

La commande privée Bouchaud _IOR('B', 0xa0, uint32_t[64]) lance un
BLIT fixe. Le pilote crée les deux textures temporaires, transfère la
mire, soumet les commandes clôturées, relit la cible et retourne les
64 pixels. Elle est bornée à un essai par démarrage. L’application
ne choisit aucune adresse DMA, ressource, dimension ou commande
VIRGL. Le pilote garde la synchronisation et la règle de restitution
des buffers seulement lorsque leur détachement est confirmé.
Les ioctls DRM d’allocation et de soumission restent à implémenter.

Les commandes privées suivantes opèrent sur des textures 8 × 8 BGRX
choisies par l’appelant. Les champs sont alignés et encodés en little
endian.

| Commande | Numéro ioctl | Argument |
|---|---|---|
| Créer | `0x800442a1` | Retour d’un identifiant `u32` |
| Détruire | `0x400442a2` | Identifiant `u32` |
| Charger | `0x401042a3` | `{u32 id, u32 réservé=0, u64 pixels}`, entrée de 256 octets |
| Relire | `0x401042a4` | Même structure, sortie de 256 octets |
| Copier | `0x400842a5` | `{u32 source, u32 cible}` |

Pour une nouvelle application, utiliser `tools/userland/bouchaud-gpu.h`.
Les ioctls `0xa6` à `0xa9` acceptent des textures BGRX8888 de 1 à
1 024 pixels de côté, initialisées à zéro, et des rectangles bornés.
Le pas utilisateur est exprimé en octets, multiple de quatre, entre
la largeur du rectangle × 4 et 4 096. La relecture rend zéro dans les
interstices entre lignes. Les pixels hors du rectangle cible d’un
BLIT restent intacts. Le contenu X est ignoré. Le dossier utilisateur
contient une application autonome qui utilise uniquement cet en-tête.

Le pilote borne le total à 16 Mio et à 8 Mio par ouverture. Il utilise
une liste VirtIO de segments DMA de 64 Kio au plus et libère tous les
segments seulement après détachement confirmé. Le compteur
d’identifiants ne recycle pas les valeurs lors d’un démarrage.
L’API historique 8 × 8 continue d’accepter les mêmes requêtes.

Chaque descripteur ouvert possède jusqu’à deux textures, dans une limite
globale de huit. Un descripteur dupliqué ou hérité conserve le même
propriétaire. Une ouverture indépendante reçoit un autre propriétaire.
Les transferts et les BLIT attendent la clôture VirtIO avant de rendre
la main. Le canal X est ignoré. Toute texture fermée sans destruction
reste dans le quota jusqu’à la prochaine allocation, qui récupère les
ressources orphelines. Après une réponse incertaine, toute page
potentiellement tenue par le périphérique reste réservée. Les ioctls
font passer des identifiants et des pixels par copie utilisateur,
avec les commandes VIRGL et les adresses DMA construites dans le noyau.

~~~bash
cargo bootimage --features p10-virgl-experimental
DISPLAY=:77 LIBGL_ALWAYS_SOFTWARE=1 tools/ci/run_virtgpu_ring3.sh \
  target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin /tmp/p10-ring3
~~~

Préparer un affichage X libre et QEMU SDL avec les modules OpenGL/VIRGL,
ImageMagick, Pillow et musl-gcc. Le banc exige les 64 pixels lus depuis
ring 3, les capsets, la mire de fenêtre et l’absence du nœud 3D sur le
bras 2D. Il vérifie aussi pointeur invalide, version inconnue, requête
de soumission DRM indisponible et limite d’un BLIT de sonde.
Deux processus effectuent chacun quatre cycles de création, chargement,
BLIT, relecture et destruction avec des motifs distincts. Le test vérifie
aussi le refus d’une ressource appartenant à l’autre ouverture, la
relecture avant initialisation, la réutilisation d’un identifiant
détruit, le quota et la récupération après fermeture. Le capset VIRGL2 v2
obtenu en TCG faisait 1 384 octets. La mire a donné 254 934 pixels exacts,
1 066 blancs attendus et zéro pixel erroné.

Le banc supplémentaire vérifie 2 376 978 pixels sur huit cycles à
64 × 64, 256 × 256, 513 × 137 et 1 024 × 1 024. Les dimensions
irrégulières exercent les lignes qui traversent un segment DMA.
Il vérifie aussi le rejet du format, du pas et des rectangles invalides.
Le test multiprocessus synchronise la création, le chargement et le
BLIT entre deux ouvertures indépendantes, puis compare 786 432 pixels
sur douze cycles. Un processus tué par SIGKILL laisse deux textures
que l’allocation suivante doit récupérer. Le banc reste à exécuter
avec une fermeture pendant une commande en vol. Répéter avec
`BO_VIRTGPU_SMP=4` pour le profil quatre vCPU.

## Mesure Linux TCG avec rendu logiciel hôte

Même noyau, QEMU, écran, mire, SMP2 et 2 048 Mio. Seule la
politique gpu-composition on/off change. Premier relevé de 30 s,
démarrage compris. Les centiles sont les classes des histogrammes
du noyau, exprimées en µs.

| Mesure | Composition CPU | Composition VIRGL |
|---|---:|---:|
| Trames bureau / VIRGL | 1 457 / 0 | 1 459 / 1 458 |
| Pixels clients copiés par le CPU | 2 522 532 | 4 928 |
| Pixels montés vers l’hôte | 0 | 2 989 832 |
| Dessin CPU moyen par trame | 1 986 µs | 1 822 µs |
| Présentation P50 / P95 / P99 | 8 192 / 16 384 / 16 384 | 4 096 / 8 192 / 16 384 |

Cette paire mesure un déplacement du travail de composition vers
VIRGL. L’activité CPU hôte, le rendu de page Ladybird et la latence
utilisateur sous WHPX n’ont pas été mesurés par ce banc.

## Vulkan, Skia et GPU physique

Un essai contrôlé avec QEMU 10.0.13, TCG SMP2, SDL/GL sous Xvfb
et Lavapipe déclaré comme ICD Vulkan hôte compare quatre lancements
avec `-trace 'enable=virtio_gpu_cmd_*'`.

| Périphérique | Bits retenus | Résultat |
|---|---|---|
| `virtio-gpu-gl-pci` | `0x0000000100000001` | Autotest 2D et capsets VIRGL lus |
| `virtio-gpu-gl-pci,blob=on,hostmem=256M` | `0x0000000100000001` | Autotest 2D et capsets VIRGL lus |
| Même périphérique avec `venus=on` | `0x0000000100000001` | QEMU bloqué sur `GET_DISPLAY_INFO` |
| Même périphérique, image expérimentale acceptant BLOB et CONTEXT_INIT | `0x0000000100000019` | Même blocage, suivi d’une assertion XCB lors de l’arrêt |

Les deux essais SDL/GL avec Venus ont nécessité SIGKILL.
En changeant uniquement le frontal par GTK/GL, l’image de banc
`banc-gpu-venus` a retenu les mêmes bits et lu le capset Venus
`id=4 version=0` de 160 octets. QEMU a confirmé l’ouverture
puis la destruction du contexte Venus. Le banc a ensuite créé
puis détruit un blob `BLOB_MEM_GUEST` de 4 096 octets. La page
DMA est conservée si une réponse est absente ou incertaine.

~~~bash
cargo bootimage --features banc-gpu-venus
# Sous un affichage X disponible et avec un QEMU/Virglrenderer compatible Venus.
timeout --kill-after=5 45 qemu-system-x86_64 -accel tcg -cpu max -smp 2 -m 2048 \
  -vga none -device virtio-gpu-gl-pci,blob=on,hostmem=256M,venus=on \
  -display gtk,gl=on -no-reboot -serial file:venus-serie.log \
  -trace 'enable=virtio_gpu_cmd_*' \
  -drive format=raw,file=target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin,snapshot=on
grep -E 'P10_VENUS_CONTEXTE_OK|P10_BLOB_INVITE_OK' venus-serie.log
~~~

Les deux marqueurs doivent apparaître. L’essai local utilise
Mesa logiciel pour GL et Lavapipe comme ICD Vulkan hôte.
Les images livrées continuent de négocier VERSION_1 et VIRGL.
Un pilote Vulkan invité exige encore des handles DRM et le
cycle complet des ressources adossées à de la mémoire Vulkan. Le test UEFI sans
périphérique VirtIO a sélectionné le GOP avec la même image.

Le banc ring 3 exécuté sur QEMU 10.0.13 puis 11.1.3, avec
virglrenderer 1.1.0 et son exécutable `virgl_render_server`, a reçu trois
réponses à `vkEnumerateInstanceVersion` par hôte. Le périphérique
annonce une fenêtre PCI hostmem de 256 Mio. Seule une page de cette
fenêtre est utilisée, dans le noyau. Un blob HOST3D mappable (ID blob
zéro) porte le flux de réponse. Le banc contrôle l’opcode, le résultat
Vulkan et la version 1.3.309 après la clôture et la publication de la
réponse. Aucun flux de commandes arbitraire n’entre par l’ioctl.

~~~bash
cargo bootimage --features banc-gpu-venus
DISPLAY=:77 tools/ci/run_virtgpu_venus.sh \
  target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin /tmp/venus-ring3
~~~

Il faut le module OpenGL de QEMU et le serveur de rendu correspondant
à virglrenderer. En installation extraite, définir
`RENDER_SERVER_EXEC_PATH` vers `virgl_render_server`. Pour reproduire
le profil logiciel, fournir le chemin de l’ICD Lavapipe avec
`VK_DRIVER_FILES` et définir `LIBGL_ALWAYS_SOFTWARE=1`.

Le profil `BO_VENUS_DEMAPPE=1` ajoute le détachement et la réouverture.
Sur QEMU 11.1.3, `RESOURCE_UNMAP_BLOB` a expiré en TCG à un et plusieurs
fils d’émulation. Sur QEMU 11.2 RC avec virglrenderer 1.3.0, le
démappage, la destruction et la réouverture ont passé avec Vulkan
1.4.309. Ce profil doit réussir le démappage pour passer.
Le profil de requête garde une seule page de transport jusqu’au reset
et les images de production excluent le banc. La requête de version
n’alloue pas de buffer de périphérique ni ne soumet `vkCmdFillBuffer`.
Mesa et le loader restent à intégrer dans l’invité.

La sonde autonome tools/userland/vulkan-transfert-probe.c fait un
vkCmdFillBuffer puis vérifie une relecture de 4 096 octets après fence.
Elle compile avec les en-têtes Vulkan et -lvulkan. Elle passe sur
l’hôte Linux avec Lavapipe et --allow-cpu. Sans cette option elle
écarte le périphérique CPU et sort 77. Ce résultat valide la sonde
logicielle sur l’hôte. La sonde reste à exécuter dans Bouchaud OS
avec un loader et un ICD appropriés.

Dans Ladybird épinglé à cdfe5f858eb5, Services/Compositor/main.cpp
conditionne la création du backend à force_cpu_painting.
LibGfx/SkiaBackendContext.cpp contient USE_VULKAN et
USE_VULKAN_DMABUF_IMAGES. Le build Bouchaud garde la peinture CPU.
Pour le raster matériel il reste à valider l’ICD, les imports et
exports de mémoire et la synchronisation explicite avant de modifier
le démarrage du Compositor. ANGLE/EGL/GLES et le rendu WebGL exigent
leur propre validation. L’interface de présentation gfx sépare déjà
GOP, VGA et VirtIO. Une branche AMD physique exige l’identifiant PCI
exact de la Trigkey, les firmwares correspondants et une validation
sur machine des opérations de mémoire et d’affichage.

La référence TRIGKEY Speed S5 déclare un Ryzen 7 5700U. L’audit GPU
ancien se fonde sur un relevé 5800H/Cezanne. Confirmer, lors d’un
même démarrage, le CPU et les identifiants vendeur/périphérique du
contrôleur d’affichage avant de choisir la génération de firmware
AMD. Voir [le manifeste de référence](TRIGKEY_REFERENCE.md).

La panne injectée à la trame 300 a entraîné le repli CPU, puis la
reprise avec 876 trames VIRGL dans le relevé suivant et la même mire
exacte. Cela exerce le refus avec file de contrôle intacte. La
perte physique du périphérique et le retour au GOP sur Windows
restent à tester.

## Fiabilité du scanout

La première suite TCG a produit une capture Xvfb 1920 × 1200
entièrement noire à 50 s. L’invité avait déjà activé le scanout,
présenté une couche GPU et produit 600 trames de mire. La série
s’est arrêtée avant le premier relevé GPU de 30 s. Trois QEMU
d’essais antérieurs étaient encore actifs dans un quota de deux
cœurs. Une reprise isolée, puis la nouvelle capture au même délai
ont donné 254 934 pixels exacts, 1 066 blancs attendus et aucun
pixel faux. La saturation est une hypothèse et la cause de l’arrêt
de la série reste à isoler.

Le banc plafonne le délai de sortie de QEMU après SIGTERM et conserve
le même noyau pour tous les bras, même si un build réécrit le fichier
source entre deux démarrages. Un second échec a montré une
collision entre deux bancs utilisant le même écran Xvfb. Le banc
réserve désormais un écran distinct pour chaque scénario et
archive les journaux QEMU et Xvfb. Les délais de capture et
critères d’acceptation restent ceux du banc.

La sortie du scanout annule le pointeur VirtIO avant de présenter
vers le GOP, en passant par le tampon arrière. Elle est déclenchée
aussi quand une trame n’a que des régions de fenêtres GPU et que
son plan de transfert 2D est vide. Ce cas de sortie est distinct
du noir intermittent. Les garde-fous DMA et VirtIO-PCI passent
avec 13 cas négatifs DMA et 31 cas VirtIO-PCI, dont les 25
existants. Les nouveaux cas vérifient l’isolement du banc Venus,
les bits écrits et la restitution de sa page DMA.

Pour le repli GOP, construire une image Stage 2 avec
`uefi-boot,reference-bringup,reference-desktop,p10-virgl-experimental,banc-gpu-perte-scanout`
et passer cette image à `tools/ci/run_virtgpu_gop_fault.sh` avec les deux
fichiers OVMF et un dossier de preuves neuf. Le banc lance une mire
ring 3, vérifie l’ordre couche GPU → perte injectée sans transfert 2D
→ retour GOP → 600 trames, puis lit l’écran VGA primaire via QMP.
Sous QEMU 11.1.3 TCG, le repli à la trame 120 a laissé 254 934 pixels
exacts et zéro pixel faux. La capture QMP porte sur le GOP après la
perte. Le passage du GPU avant la perte est vérifié dans la série.
Le banc injecte une file cassée et exige un redémarrage pour retrouver
VirtIO après ce type de perte. Sur un des lancements QEMU 10.0.13/GTK,
le processus a échoué sur EGL_BAD_ACCESS avant le scanout de l’invité.
Le premier écran noir de la campagne antérieure reste à isoler.
