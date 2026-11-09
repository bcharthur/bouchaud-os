# P10 GPU — report sur main (9 novembre 2026)

Base: `6bdcb36f3432d202d37b4562f43dee7363d0852f`; branche `feat/p10-mainline-gpu-20261109` ; aucune modification DOOM historique.

- Capset VIRGL et contexte 3D réellement négociés ; commandes fences et contrôle des réponses.
- Upload d'une surface cliente ring3 vers texture via DMA, BLIT VIRGL, un seul RESOURCE_FLUSH complet, retour CPU.
- Travailleur de présentation asynchrone avec snapshots réutilisables et statistiques de pertes.
- Branchement P10 dans `widgets::compose_client` et `window_manager` versionnés dans cette branche.
- Sélection de `Mode::Copie` spécifique au P10, **sans** toucher à `TAMPON_DIRECT_VOULU`. P10+.2 garde sa politique.
- Compilation optionnelle des dépendances Vulkan pour Ladybird ; **aucun GPU Skia Vulkan ring3 encore prouvé**.

Limites: seulement une fenêtre sans occlusion, QEMU VIRGL expérimental, ni pilote AMD physique ni performance démontrée. Ne pas fusionner sans CI et QEMU pixels/latence.

## P10 multi-fenêtres sans recouvrement — étape 2

Le compositeur GPU accepte désormais jusqu'à **4 surfaces clients disjointes** dans une trame. Le WM interdit le chemin si deux fenêtres se recouvrent ; la file conserve leur ordre, le pilote réalise une séquence DMA / BLIT par surface, puis effectue **un seul flush plein écran**. Le repli CPU reconstruit tous les clients d'une trame rejetée.

Il s'agit d'une composition GPU réelle mais expérimentale : les surfaces chevauchées et le blending transparent restent sur le chemin CPU. La rasterisation Skia dans Ladybird n'est pas transférée sur Vulkan tant que le pilote invité n'existe pas.

## Lancement de la branche GPU en isolation du worktree Windows

```powershell
cd "C:\Users\Arthur\RustroverProjects\bouchaud-os-doom"
git fetch origin
git worktree add "C:\Users\Arthur\RustroverProjects\bouchaud-os-p10-mainline" origin/feat/p10-mainline-gpu-20261109
cd "C:\Users\Arthur\RustroverProjects\bouchaud-os-p10-mainline"
.\run.ps1 -Doom -P10GPU -Accel tcg -CpuCount 4 -RamMiB 4096
```

`-P10GPU` construit le noyau avec `p10-universal-present,p10-virgl-experimental` puis exige un QEMU avec `virtio-gpu-gl-pci` et sortie OpenGL. Le lancement sans `-P10GPU` ne modifie pas le comportement 2D. Vérifier les logs `P10_GPU_3D_OK`, `P10_GPU_CLIENT_BLIT_OK couches=2..`, les pixels, la fluidité et les refus du GPU avant qualification.

## P10 — soumission groupée de quatre textures (9 novembre 2026)

Les sources de `p10_window_gpu.rs` exposent un pool borné de quatre identifiants de ressources VIRGL privés. Chaque emplacement de fenêtre possède un backing DMA 3D réutilisable. Le pilote transfère chaque surface sur **sa texture distincte**, encode ensuite la scène ordonnée et n'émet qu'**un seul `SUBMIT_3D`** pour les BLIT des fenêtres, puis un seul flush. L'ancien chemin réutilisait la même texture et imposait un `SUBMIT_3D` par fenêtre afin d'éviter les lectures écrasées.

Tests hôte : allocation et bornage des quatre identifiants, paquet 4 BLIT correctement ordonné, rejet complet d'une scène invalide. CI Fast compile le noyau avec `p10-universal-present,p10-virgl-experimental` ; les tests de transport DMA P10+.2 sont conservés. Les gains de temps et la visibilité des pixels attendent des mesures QEMU sur ce commit.

**Consommation mémoire** : jusqu'à 4 backings DMA de la taille du scanout peuvent rester alloués pendant la session graphique, contre un backing auparavant. C'est un compromis explicite entre mémoire GPU et nombre de soumissions 3D ; ce pool n'est pas exposé aux processus ring 3.
