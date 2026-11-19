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
