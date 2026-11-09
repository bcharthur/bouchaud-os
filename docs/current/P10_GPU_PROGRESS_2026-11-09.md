# P10 GPU — état expérimental et préparation du commit (9 novembre 2026)

> **Qualification, pas release.** Cette page décrit des travaux installés dans une
> copie locale Windows du dépôt et observés sous QEMU-TCG + VIRGL. Ces changements
> **ne sont pas nécessairement présents sur `main` ni sur cette branche GitHub**.
> Le statut "code présent localement" ne signifie pas "GPU complet", ni "CI verte".
>
> Sources de cette qualification : journaux utilisateurs du 9 novembre 2026,
> série de patchs locaux P10, compilations Rust locales et tests unitaires isolés.
> Base locale signalée au démarrage : `fix/doom-logo-officiel-20261105`,
> `9a8b1dcb`, avec **99 fichiers non committés** au lancement de 21:39.
> La branche documentaire ne contient **aucun code P10 local récupéré**.

## Mise à jour GitHub du 9 novembre, 20:09 UTC

- Le prototype P10 a été sauvegardé depuis la machine Windows dans le commit **`816969a274da35826536e6d28c970b718eef6352`** sur **`feat/p10-gpu-preparation-20261109`**. La **[PR #280](https://github.com/bcharthur/bouchaud-os/pull/280)** est **en brouillon, non fusionnable en l'état**. Ne pas confondre la sauvegarde Git et la qualification de P10.
- Le commit contient **9 fichiers** : cœur VirtIO-GPU, file de présentation et modules P10, et préparation facultative Vulkan côté vcpkg. Il ne contient **ni `Cargo.toml` (flags `p10-virgl-experimental`, `p10-universal-present`), ni raccordement P10 de `src/gui/widgets.rs`, ni mise à jour de `run.ps1`**, qui demeurent localement modifiés.
- La branche issue de l'ancien snapshot contient aussi des changements DOOM historiques et se situe à **59 commits derrière `main`**, contre 3 commits propres au moment de la comparaison. Une extraction des seuls changements P10 sur base récente sera nécessaire avant merge.
- Les expériences QEMU précédentes provenaient d'un **arbre local non committé** ; elles ne prouvent pas que la PR #280 est auto-suffisante ou déployable en CI.

## Bilan technique au 9 novembre

| Sous-système | État de preuve | Réalisation / limite |
|---|---|---|
| VirtIO-GPU 2D | existant sur le dépôt, qualifié auparavant en QEMU | scanout, transfert, flush, IRQ ; aucune rasterisation GPU |
| Initialisation VIRGL | **observé QEMU-TCG local**, expérimental | capset 2 / version 2 ; `P10_GPU_3D_OK` avec contexte et ressource 64×64 |
| Présentation universelle | **observé QEMU-TCG local**, expérimental | `p10-gpu-present` : une file de snapshots avec remplacement de trames obsolètes |
| Surfaces applicatives sur VIRGL | **observé QEMU-TCG local**, expérimental | `P10_GPU_CLIENT_BLIT_OK` : texture DMA 3D → commande BLIT → FLUSH, fenêtre visible |
| Correctif orientation et affichage unique | **code appliqué et compilé localement**, instrumentation observée | retournement de lignes source, `orientation=top-down present=atomique-1-flush` ; confirmation visuelle complète et tests de déchirure encore exigés |
| Composition de plusieurs fenêtres par GPU | **non qualifiée** | chemin accéléré limité à une fenêtre admissible ; règles de recouvrement, curseur et transparence à démontrer |
| Peinture Skia accélérée | **non qualifiée / non démontrée** | navigateur chargé depuis `native-browser-m9` précompilé ; aucune preuve de backend Skia Vulkan matériel, raster CPU encore présent |
| GPU AMD physique Trigkey | **non testé** | QEMU VIRGL ≠ pilote AMD natif |

### Mesures observées (exécution du 9 novembre vers 21:39)

- Compilation du noyau réussie via `run.ps1` (cargo nightly 1.98).
- Cinq tests Rust du module P10 orientation réussis (dont `inversion_des_lignes_gpu_sans_perte`).
- `P10_GPU_3D_OK capset=2 version=2 ...`.
- `P10_GPU_UNIVERSAL actif=1`, plan `cpu-backbuffer->snapshot->gpu-worker->virtio-scanout`.
- `P10_GPU_CLIENT_BLIT_OK trames=1` à 21:39:21, puis `trames=64` à 21:39:49 ;
  `source=1100x566 destination=90,69 chemin=DMA-3D-BLIT-FLUSH orientation=top-down present=atomique-1-flush`.
- Dernier relevé `P10_GPU_QUEUE` : **834 soumises, 448 présentées, 386 remplacées,
  saturation=0, échecs=0, moyenne=29 668 µs**. Le compteur de trames utiles
  descend ponctuellement sous 10 FPS lors du chargement ; ce n'est **pas**
  une amélioration de performance vérifiée.
- Absence de `panic`, `error:` ou `VIRTIO_GPU_SCANOUT_ABANDON` dans
  **l'extrait fourni**. Cela ne garantit pas l'absence de défaut sur un essai long.

### Incident connu et correction locale

L'essai initial a montré un contenu Ladybird **inversé verticalement**, avec
clignotement du fond et du contenu alors que la bordure Atelier était correcte.
Diagnostic : convention d'orientation de la texture VIRGL, combinée à
deux présentations successives (fond 2D puis texture 3D). La correction locale
`BOUCHAUD-P10-ORIENTATION-ATOMIC-20261109` remanie le transfert et le
flush en une présentation finale, et réordonne les lignes de la texture
source. Les journaux prouvent que le chemin corrigé est **appelé**, pas que
l'image a été jugée conforme pixel à pixel.

## Répartition des composants dans la copie locale

Les noms ci-dessous sont des **points d'inventaire**, pas une liste
d'engagement à committer en totalité :

- `src/drivers/display/virtio_gpu.rs` : transport/commandes 2D et 3D, scanout.
- `src/drivers/display/bochs.rs` : présentation et transition graphique.
- `src/drivers/display/p10_virgl_composition.rs` : encodage de BLIT / scènes.
- `src/drivers/display/p10_window_gpu.rs` : ressources/texture 3D de fenêtre et orientation.
- `src/drivers/display/p10_gpu_queue.rs` : travailleur et file de présentation.
- `src/drivers/display/p10_surface_contract.rs` : validation de surfaces si présent.
- `src/drivers/mod.rs` : déclarations de modules.
- `src/gui/widgets.rs` et `src/gui/window_manager.rs` : branchement des surfaces et du gestionnaire de fenêtres.
- `tools/ladybird/browser-vcpkg.sh` et `tools/ladybird/ui-bouchaud/main.cpp` :
  préparation Vulkan **conditionnelle** ; l'artefact navigateur précompilé
  reste à reconstruire pour tout changement natif.

Attention : ces fichiers peuvent aussi porter d'autres travaux non P10.
Réviser les hunks avant staging : ne pas committer en bloc 99 fichiers.

## Règles d'intégration

1. **Inventaire avant commit** : `git status --porcelain=v1`,
   `git diff --stat`, `git diff --check`, `git diff --cached`,
   `git ls-files --others --exclude-standard`.
2. Créer une **branche de travail P10** depuis le HEAD local sans rebase,
   reset ni nettoyage des modifications existantes ; ne pas toucher à
   `main` tant que les tests ne sont pas qualifiés.
3. Ajouter uniquement les nouvelles sources P10 nécessaires ; utiliser
   `git add -p` pour les fichiers partagés (widgets, window manager,
   pilote commun, outils Ladybird).
4. Compilation noyau **sur le commit exact** ; tests protocolaires Rust ;
   boot QEMU-TCG 2D et QEMU-TCG VIRGL ; preuve visuelle côté invité/host.
5. Contrat de non-régression : correctitude pixel à pixel, bordures/curseur,
   superposition, DOOM et Ladybird, redimensionnement/déplacement,
   sorties/rentrées du mode graphique, arrière-plan/avant-plan, détection
   trames mixtes et pertes GPU, mesure p50/p95 et FPS utiles.
6. Exiger KVM/WHPX selon disponibilités, puis tests Trigkey séparés.
   **N'annoncer aucune accélération Skia sans preuve du backend GPU réellement
   choisi et sans benchmark pertinent.** Aucun assouplissement des CI.

## Qualification suivante

- Confirmer par capture que le correctif top-down ne retourne plus la page.
- Résoudre les 29,7 ms de présentation moyenne et les 386 remplacements /
  834 soumissions ; comparer à VirtIO-GPU 2D dans un protocole A/B identique.
- Généraliser les calques GPU à plusieurs fenêtres avec ordre Z, masques,
  opacité et fences, ou revenir à la composition CPU si non sûre.
- Implémenter un chemin de rendu Skia GPU réellement utilisable par Ladybird
  côté **ring 3**, avec interfaces Vulkan/DRM/partage mémoire appropriées ;
  éliminer `--force-cpu-painting` seulement si le backend fonctionne.
- Isoler la migration GPU du noyau/du navigateur du reste des ~99 modifications.

**Statut de cette page :** documentation de preuves locales et garde de
préparation. Elle n'autorise ni fusion automatique dans `main`, ni
affirmation que P10 est terminé.
