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
