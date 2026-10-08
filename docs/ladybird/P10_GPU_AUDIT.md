# P10 — GPU : audit, verdict BLOCKED, premier composant

> État au 2026-10-08, HEAD `c63a328e`, Ladybird épinglé
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
| `-device virtio-gpu-pci` sous QEMU | énuméré (`[PCI-NG] peripheriques=7` au lieu de 6), aucun pilote ne s'y attache, le bureau reste sur BGA | [QEMU-TCG] boot local du 2026-10-08 |
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

Un **virtio-gpu 2D** qui affiche le bureau à la place de BGA serait l'étape
suivante. Il ne ferait pas passer P10 en DONE, puisqu'il n'y a toujours pas
d'accélération. Il faut le dire comme ça, et ne pas le présenter autrement.

## 5. Ce que P10 ne prétend pas

- Il n'y a ni WebGL, ni WebGPU, ni canvas accéléré. Le canvas 2D est peint en
  CPU par Skia, dans le Compositor.
- Aucune mesure n'a comparé le raster CPU à un GPU. Le débit d'affichage
  mesuré par l'endurance (latence d'un cadre p50/p95/p99) est celui du
  raster CPU.
- `virtio-gpu (QEMU) ≠ GPU de la Trigkey` : une preuve sous QEMU ne
  vaudrait jamais pour la machine physique.
