# Passation Codex — PR #348

Branche `codex/p10-real-gpu-vulkan-20261119`, reprise depuis `9233071a`.
La PR reste en brouillon. Travail GPU uniquement sur cette branche.

## `caf4ecc74e8860c05e0455cda33f84c4ffaaac1e` — taille du blob de réponse

- Le banc privé Venus accepte de 4 Kio à 1 Mio, par multiples de page, dans la limite de l'ouverture PCI. Une seule page est accessible au noyau pour lire la réponse.
- QEMU 11.2 RC, virglrenderer 1.3.0, Lavapipe et GTK en TCG. Quatre cycles 4 Kio / 64 Kio / 1 Mio / 4 Kio passent, ainsi que les refus de taille invalide et de changement de taille à chaud. Garde VirtIO-PCI 33/33.
- QEMU 11.1.3 et virglrenderer 1.1.0. La trace atteint `RESOURCE_UNMAP_BLOB mapping_state=0`, sans accusé. L'invité expire après 1 000 858 µs, puis condamne le canal. La comparaison des versions ne suffit pas à attribuer la cause à une seule bibliothèque. Ne pas réessayer après une issue DMA incertaine.
- CI GitHub à rattacher au SHA du push. Le banc Venus exige un QEMU et un serveur de rendu compatibles.
- Suite immédiate. Rattacher le canal à l'ouverture du descripteur et exercer la récupération après la mort du client.

## Rejouer le banc

Construire avec `cargo bootimage --features banc-gpu-venus`. Sous Xvfb avec GTK/GL et un serveur Venus compatible :

```sh
BO_VENUS_DEMAPPE=1 tools/ci/run_virtgpu_venus.sh target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin /tmp/bouchaud-venus
```

`vkCmdFillBuffer`, l'ABI DRM générale, Mesa et Skia restent ouverts.

## `d8869f91` — isolation du canal et récupération

- Le canal est lié à une description de fichier ouverte. Une autre ouverture ne peut ni soumettre ni fermer le canal actif.
- QEMU 11.2 RC sous TCG. Le parent obtient EBUSY sur les deux opérations pendant que l'enfant est vivant. Après SIGKILL et waitpid, il récupère le blob orphelin, ouvre un canal neuf et vérifie la version. L'expiration sur le backend lent continue à condamner le canal.
- CI de la PR à vérifier par SHA. Reste à exercer la mort pendant une commande effectivement en vol.
- Suite immédiate. Servir le capset Venus via les requêtes DRM standard GETPARAM et GET_CAPS.

## `979836d9ea80d70343af083e357edf7dd802927b` — découverte DRM

- QEMU 11.2 RC TCG. GETPARAM identifie le capset Venus et GET_CAPS en lit le contenu. Le banc rejette une version de capset incorrecte. La découverte VIRGL conserve son chemin normal.
- Ce commit expose les requêtes de découverte. Suite immédiate. Créer un contexte détenu par l'ouverture via CONTEXT_INIT et valider le refus de réinitialisation.

## `9fdf7c079cb5946a568d25fe6eda0a6b2eb87885` — contexte DRM

- CONTEXT_INIT crée un contexte Venus réel à un anneau, détenu par l'ouverture. Le banc 11.2 RC vérifie le rejet d'un second anneau, d'un pointeur invalide et d'une réinitialisation avec EEXIST, puis réutilise le contexte pour la soumission.
- Contre-épreuve avec QEMU 11.1.3, virglrenderer **1.3.0** et le même noyau invité. La version 1.4.309 est lue, mais UNMAP expire à 1 001 201 µs. La mise à jour de virglrenderer seule ne suffit donc pas dans ce montage. Le chemin asynchrone QEMU reste à isoler.
- Suite immédiate. Vérifier l'identité du contexte dans la clôture renvoyée puis rejouer le profil normal VIRGL.

## `088031540c901809fe829f7bea69143d7aacbd42` — clôture contextualisée

- Un retour clôturé dont le contexte diffère de la requête INFO_RING_IDX est traité comme une réponse invalide. Le chemin classique garde ses critères.
- 14 tests de la file passent. Banc complet Venus 11.2 RC vert.
- Suite immédiate. Exercer des commandes sérialisées par le processus via le sous-ensemble DRM EXECBUFFER, puis développer l'ABI GEM et mmap nécessaire à Mesa.

## `3aa517dfb17cdc2825c2f80e02496753b2acabfc` — soumission DRM

- EXECBUFFER transmet les mots Venus encodés par l'application dans son contexte. Profil synchrone à un anneau, taille bornée, sans syncobj, sans descripteur de fence et sans liste GEM. Une copie privée de 64 octets lit le blob de réponse. Le banc reste hors de la configuration par défaut.
- QEMU 11.2 RC / TCG. Deux `vkCreateInstance` et `vkDestroyInstance` émis et vérifiés côté application, reprise du blob après SIGKILL et démappage vert. Le capset et le contexte passent aussi les vérifications précédentes.
- Suite immédiate. Énumérer les périphériques côté application, puis préparer VkDevice et un buffer. La réponse privée reste à remplacer par GEM et mmap pour Mesa.

## `e0117451` — énumération Vulkan

- Sous QEMU 11.2 RC avec Lavapipe, deux instances distinctes relisent un appareil et le handle attendu par `vkEnumeratePhysicalDevices`.
- Suite immédiate. Créer une queue et un VkBuffer depuis le programme, puis utiliser les besoins mémoire de ce buffer pour préparer l'allocation et la clôture.

## `a212a3ca` — périphérique logique et buffer

- Le programme vérifie la capacité de transfert de la queue 0, crée VkDevice puis VkBuffer de 4 096 octets et vérifie les besoins mémoire. Deux cycles réussis sur QEMU 11.2 RC avec Lavapipe.
- Suite immédiate. Allouer et lier la mémoire, enregistrer et soumettre vkCmdFillBuffer, puis attendre une vraie VkFence. Pour la comparaison des octets, ajouter un second blob HOST3D lié à VkDeviceMemory sans perturber le blob de réponse.

## `70eb14c8` — vkCmdFillBuffer et VkFence

- L'application choisit un type de mémoire cohérent et visible par l'hôte, lie VkDeviceMemory, enregistre vkCmdFillBuffer, soumet avec VkFence et vérifie VK_SUCCESS à vkWaitForFences. Deux cycles sous QEMU 11.2 RC avec Lavapipe.
- vkGetDeviceQueue est rejeté par Venus. Le chemin qualifié utilise vkGetDeviceQueue2 et VkDeviceQueueTimelineInfoMESA.
- Suite immédiate. Exporter la VkDeviceMemory vers une seconde fenêtre hostmem, relire et comparer les 4 096 octets avant de détruire les objets.

## Isolation EXECBUFFER et réponse

- Le banc vérifie qu'une autre ouverture reçoit EBUSY à EXECBUFFER et LIT_REPONSE, avec une réponse de 512 octets intacte, avant le SIGKILL et la récupération du canal. Banc complet vert sous QEMU 11.2 RC / TCG / Lavapipe, y compris les deux clôtures Vulkan.
- Non-regression Gate vert sur 70eb14c8 (44625407328). GPU composition 44625407341 était en cours au contrôle.
- Relecture expérimentale. QEMU confirme CREATE_BLOB et MAP_BLOB du blob 0xb108 associé à VkDeviceMemory 0x401. UNMAP_BLOB entre dans le chemin asynchrone (mapping_state 0) puis ne répond pas. Même résultat sans accès CPU et avec TCG monothread. Le blob de commande du chemin MAP_FIXED se démonte. Le prototype est conservé hors branche dans `/workspace/work/p10/venus-readback-repro.patch`.
- Reprise précise. Inspecter la dernière référence à la MemoryRegion de 0xb108 et le rappel `virtio_gpu_virgl_hostmem_region_finalize` dans QEMU 11.2 RC, puis relancer `cargo bootimage --features banc-gpu-venus` et `DISPLAY=:N BO_VENUS_DEMAPPE=1 tools/ci/run_virtgpu_venus.sh BOOTIMAGE SORTIE_NEUVE`. La qualification de la relecture exige deux motifs sur tous les 4 096 octets et le démappage confirmé.

## Reprise du contexte avant création de blob

- Le contrôle de propriétaire renvoie désormais EBUSY dès CONTEXT_INIT, y compris avant la création de la fenêtre de réponse. Le banc tue les deux formes de propriétaire (avec et sans blob) et valide que la prochaine ouverture recrée et démonte le canal.
- Banc Venus complet vert sous QEMU 11.2 RC / TCG / Lavapipe. Les deux marqueurs de récupération sont présents. Les 216 garde-fous d'architecture passent sur le lot précédent.

## `f2f18d4b172067697120c3086c3f3d090a901aee` — validation et diagnostic de relecture

- Compilation de l'image par défaut et du banc Venus. VIRTGPU_RING3_OK en SMP2 et SMP4 sous TCG avec VirGL logiciel, y compris capture de mire (254 934 pixels exacts et 0 pixels faux dans chaque capture), textures et repli 2D. Non-regression Gate 44626700794 vert. GPU composition 44626700649 en attente au contrôle.
- Premier motif comparé côté noyau dans le prototype, avant le démappage. `P10_VENUS_DIAGNOSTIC_MEMOIRE octets=4096 differences=0`, puis `P10_VENUS_RELECTURE_ECHEC etape=unmap-memoire-sans-reponse`. Le banc doit donc rester en échec avec ce prototype. Le programme ring 3 n'a pas reçu le contenu et la deuxième comparaison n'a pas eu lieu.
- Traces locales de la contre-épreuve dans `/workspace/work/p10/resume-readback-diagnostic`. Vérifier les délais côté hôte et les références MemoryRegion avant d'envisager tout changement de stratégie de mappage. Conserver la quarantaine si l'issue d'une commande reste inconnue.

## Relecture de deux motifs Vulkan depuis ring 3

- QEMU place une commande suspendue qui porte FENCE dans fenceq alors que RESOURCE_UNMAP_BLOB s'est arrêté avant la création de la fence. Le finaliseur libère la MemoryRegion retraitée par RCU et relance cmdq. La commande clôturée a déjà quitté cmdq. La trace du chemin corrigé montre les états 0 puis 2 pour chaque export.
- Le banc omet FENCE exclusivement pour les deux requêtes exactes UNMAP des blobs HOST3D sans pages DMA prêtées. Il vérifie le retour utilisé et l'en-tête, puis exige UNREF clôturé avant de recycler la ressource. Les commandes de production restent clôturées et l'issue inconnue entraîne la quarantaine.
- Deux motifs, 5a174101 et 5a174102, comparés sur les 4 096 octets chacun par le processus utilisateur. Création, soumission, VkFence, relecture, destruction et réouverture passent sur QEMU 11.1.3 et 11.2 RC, TCG avec Lavapipe. Contre-épreuve 11.1.3 avec virglrenderer 1.3.0 également verte. 15 tests de la file et garde-fous VirtIO passent.
- Suite. Comptabiliser les objets par ouverture, introduire CREATE_BLOB, RESOURCE_INFO et GEM_CLOSE DRM, puis exposer MAP/mmap avec une durée de vie liée au mapping. Rejouer les appels Venus de Mesa avant d'activer Skia.

## `3ef811ca31a13d7f12235ded4da9a42ae3401c71` — jalon relecture, puis premiers objets GEM

- Non-regression Gate 44699161137 vert. CI GPU 44699161026 en attente lors du premier contrôle. Trois bancs de lecture verts en TCG. Les neuf démontages du blob de réponse observés sur QEMU 11.1.3 rendent OK entre 0,558 et 11,029 ms lors de ce passage. Avant, la première commande expirait à environ 1 s. Ces durées ne mesurent pas Ladybird.
- Les commandes DRM standard CREATE_BLOB, RESOURCE_INFO et GEM_CLOSE associent un handle logiciel monotone à une vraie ressource HOST3D. Huit slots bornés, libérés après UNREF clôturé, contrôle par ouverture et récupération des objets après la mort du propriétaire. QEMU 11.2 RC valide quota et réutilisation. QEMU 11.1.3 valide en plus le rejet de l'accès au GEM de l'enfant et la récupération du quota complet après SIGKILL.
- Suite. Étendre le backing de périphérique du sous-système VMA aux mappings GEM, en liant leur référence au retrait confirmé des pages et aux clones après fork. Les indicateurs DRM HOST_VISIBLE et RESOURCE_BLOB restent à zéro avant le chemin MAP/mmap. Mesa demande aussi plusieurs anneaux et la synchronisation DRM.

## `69775d3c` — corps des objets GEM, puis DRM VERSION

- Ajout de VERSION x86_64 avec lecture des longueurs, copie bornée et contrôle EFAULT. Vérifié en ring 3 sous QEMU 11.2 RC, avec le banc complet relecture et GEM.
- Le Vulkan Loader Debian 1.4.309 s'exécute dans l'invité, charge le manifeste puis le vrai ICD Mesa Venus. vkCreateInstance réussit. vkEnumeratePhysicalDevices échoue avec VK_ERROR_INITIALIZATION_FAILED car libdrm ne peut pas encore énumérer le nœud DRM. La reproduction locale est `bash /workspace/work/p10/run-loader-guest.sh /workspace/work/p10/SORTIE_NEUVE`.
- Suite. Exposer la découverte du nœud par libdrm à partir de l'identité PCI réelle, puis traiter les paramètres requis, les mappings et les synchronisations. La preuve retenue exige toujours la lecture complète du buffer via les fonctions publiques Vulkan.

## `48674045` — découverte libdrm

- Le banc expose à libdrm le nœud de rendu et la vue PCI de la fonction réellement sondée. La sonde ring 3 vérifie getdents, stat, readlink et PCI_SLOT_NAME. Le banc Vue PCI, GEM et relecture Vulkan passe sous QEMU 11.2 RC.
- Avec le vrai Loader et l'ICD Debian, Mesa écrit désormais `using DRM device /dev/dri/renderD128`, puis `required kernel param 3 is missing`. Le point bloquant est passé de l'énumération du nœud au support de l'ABI blobs et mappings.
- Suite. Ajouter les baux GEM détenus par les VMA avec transfert du bail à la retraite TLB lors du dernier retrait de la VMA. Vérifier explicitement munmap partiel, MAP_FIXED, fork, exec et destruction avant d'activer HOST_VISIBLE. Rejouer la sonde publique `tools/userland/vulkan-transfert-probe.c` avec l'ICD Mesa.

## `b4ec90ee` — banc des fonctions publiques Vulkan

- Le mode `BO_VENUS_LOADER_SYSROOT` prend un dossier d'invité contenant `vulkan-probe`, `venus.json`, `lib/libvulkan.so.1`, l'ICD Mesa et les dépendances de son ELF. Il exige la découverte DRM et deux comparaisons complètes. Rejouer avec `DISPLAY=:N BO_VENUS_LOADER_SYSROOT=/chemin/guest tools/ci/run_virtgpu_venus.sh BOOTIMAGE SORTIE_NEUVE`.
- La sonde publique passe les deux motifs en contrôle Linux hôte avec Lavapipe. Dans Bouchaud OS sous QEMU 11.2 RC, le verdict reste en échec à vkEnumeratePhysicalDevices (paramètre DRM 3 absent). Le banc du transport filaire reste vert. Rootfs de reproduction local dans `/workspace/work/p10/resume-guest-discovery/guest`.

## `c6f61849` — DRM MAP et mmap des blobs HOST3D

- Le BAR est borné par slot. MAP expose un offset par handle. mmap associe le contexte et son GEM à un bail d'espace d'adressage. Les fragments et retraits concurrents gardent leurs références jusqu'à l'ACK TLB, y compris avec fork et exec. GEM_CLOSE masque le handle immédiatement et le pilote ne recycle le slot qu'après la dernière référence puis UNMAP et UNREF confirmé. PROT_EXEC et l'agrandissement mremap du mapping sont refusés.
- QEMU 11.1.3 et 11.2 RC, TCG/Lavapipe. Deux motifs vérifiés intégralement par mmap DRM, en plus des deux cycles privés conservés. Tests positifs MAP_SHARED après fermeture GEM, mprotect read-only, fork, exec, munmap partiel et remplacement MAP_FIXED. Tests négatifs taille, offset, exécution et handle expiré. Garde VirtIO à 36 mutations.
- Le Vulkan Loader avec Mesa franchit maintenant RESOURCE_BLOB et HOST_VISIBLE. Le point bloquant suivant observé est `failed to initialize context: Invalid argument` sur le CONTEXT_INIT Mesa à 64 anneaux.

## `a6eda479` — contexte DRM et réponses appartenant au client

- CONTEXT_INIT valide jusqu'à 64 anneaux avec masque d'événements nul. EXECBUFFER utilise l'anneau demandé et le contexte propriétaire. Le pilote laisse le client gérer le flux de réponse dans son GEM quand le blob privé du banc n'est pas ouvert.
- Le banc valide une requête de version intégralement via le blob client. QEMU 11.1.3 et 11.2 RC verts. Les 216 garde-fous passent sur le jalon mmap.
- Le Loader et Mesa s'attachent au renderer, énumèrent le périphérique et créent le device et le buffer. La sonde échoue ensuite à vkBeginCommandBuffer avec VK_ERROR_OUT_OF_HOST_MEMORY. Mesa réserve 8 Mio pour le pool de flux indirect alors que le slot GEM est borné à 1 Mio.
- Suite. Agrandir les slots GEM selon la taille BAR en conservant un plafond, n'installer l'alias noyau que sur les pages effectivement demandées, puis retester la soumission et les fences Mesa.

## `d854867e` — pool de commandes Mesa

- Les slots GEM sont dimensionnés dans la BAR et plafonnés à 16 Mio. L'alias noyau est créé à la première projection du blob. Une allocation de 8 Mio avec écriture aux deux extrémités, les relectures et le cycle GEM passent sur QEMU 11.1.3 et 11.2 RC.
- Sur QEMU 11.2 RC/Lavapipe, la vraie sonde Vulkan Loader/Mesa a vérifié les 4 096 octets du motif 5a17c3e9. Au second cycle dans le même processus, le host a recréé le contexte et ses blobs. Mesa est ensuite resté en attente du numéro de séquence de l'anneau. Le banc public échoue donc encore.
- Suite. Répliquer le blocage sur 11.1.3, tracer le second cycle autour de la publication du flux indirect et isoler la réutilisation des identifiants de contexte et des slots avant d'activer Skia.

## `34ef1694` — validation publique et ouverture non privilégiée

- La sonde Vulkan Loader/Mesa a comparé les deux motifs sur QEMU 11.1.3 et sur une reprise 11.2 RC. La première exécution RC reste comptée comme un échec de stabilité.
- Le nœud de parcours et le périphérique synthétique ont maintenant le même mode 0666. Le banc confirme l'ouverture O_RDWR et DRM VERSION après passage à uid/gid 65534. Le pilote conserve l'isolement des handles par ouverture.
- Le build navigateur Vulkan bloque sur le téléchargement Chromium zlib épinglé par ANGLE, réponse CONNECT 403. Le build Skia séparé a repris après reconstruction de l'archive Automake depuis Debian avec le SHA512 exact de vcpkg.

## `cf355b00` — accès DRM limité au Compositor

- Le profil sandboxé de l'image installée `/usr/libexec/ladybird/Compositor` peut découvrir et ouvrir l'unique nœud Venus. L'exception de lecture est limitée aux attributs libdrm. Les mutations et les périphériques privilégiés restent refusés.
- Le banc 11.2 RC teste le chemin exact, l'accès interdit à WebContent et à un Compositor hors du répertoire prévu, la suppression, la troncature et l'accès e/s. Le banc se termine par poweroff sans raccourcir le délai de protection.
- Campagne Loader 11.2 RC. Trois réussites sur cinq lancements connus. Un échec attendait la séquence de l'anneau au second motif et un lancement TCG a expiré avant toute série. QEMU 11.1.3 a confirmé les deux motifs lors du premier essai public.

## `0136863390da0ab3cc9f5794b9dae06587455576` — profil Skia du navigateur

- Le vrai Loader/Mesa a confirmé les deux motifs avec le binaire de sonde exécuté sous l'identité sandboxée du Compositor. Cette vérification couvre l'accès DRM et le transport, indépendamment de Skia.
- Le profil de build Vulkan sépare les binaires dynamiques et maintient un profil CPU dans le même artefact. Le chemin Skia vérifie le succès de readPixels avant de marquer une relecture. Le repli à l'allocation dessine dans le bitmap partagé. Le smoke exige la relecture Vulkan et les preuves visuelles existantes.
- Préparateurs testés deux fois sur Ladybird cdfe5f858eb5, diff stable, scripts Bash vérifiés, 22/22 contrats Ladybird, mauvais mode du smoke refusé. Le build complet ne peut pas encore fournir un binaire, car ANGLE n'a pas accès à la source zlib Chromium épinglée. Skia séparé en cours via archives dont les empreintes vcpkg sont conservées et le miroir Git FreType épinglé.
- Suite. Finir le build Skia, résoudre le téléchargement ANGLE, puis lancer les deux bras du smoke avec le même artefact et relever `[LB:SKIA]`. L'échec RC de l'anneau Mesa et le rendu Vulkan de pages restent à qualifier.
