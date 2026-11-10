# P10 — compositeur GPU des fenêtres (`BOUCHAUD_GPU_COMPOSITION_V2`)

> Mis à jour le 10 novembre 2026, branche `claude/c22-p13-p10-gpu-kernel-0y2ucg`.
> Ce document distingue ce qui est **prouvé** (avec son niveau : HOST, TCG,
> KVM) de ce qui ne l'est pas. Il remplace, pour la composition, le chemin
> « présent universel » de la PR #281.

## 1. Ce qui est fait, en une phrase

Quand l'hôte propose VIRGL (QEMU `virtio-gpu-gl-pci`), **le contenu de toutes
les fenêtres ring 3 (Ladybird, DOOM, toute application du protocole GUI) est
composé par le GPU de l'hôte** : chaque surface cliente devient une texture
adossée à ses propres pages, l'hôte y monte le seul rectangle abîmé, et un
`BLIT` GPU la pose dans le scanout. Le processeur invité ne recopie plus un
seul pixel client ; il ne peint que le bureau (fond, cadres, dock, curseur), et
seulement ce qui change.

## 2. Partage du travail CPU / GPU

| Travail | Avant (CPU seul) | Avec le compositeur GPU |
|---|---|---|
| Contenu des fenêtres clientes | recopié ligne à ligne par le CPU invité dans le tampon du bureau, à chaque trame client | **aucune copie CPU** : `TRANSFER_TO_HOST_3D` du rectangle abîmé (copie faite par l'hôte depuis les pages du client), puis `BLIT` GPU |
| Fond, cadres, dock, menus, curseur | CPU, composition directe dans la mémoire du scanout | inchangé (CPU), seulement les rectangles abîmés |
| Pixels d'une fenêtre sous une ombre, le curseur, le dock | CPU | CPU, **au pixel** : le GPU pose la zone moins ces calques (`soustrait`) |
| Présentation | `TRANSFER_TO_HOST_2D` + `RESOURCE_FLUSH` | les mêmes transferts du bureau, puis **un** `SUBMIT_3D` (tous les BLIT), puis **un** flush |

Les attentes de réponse se font par interruption (P10+.1) : pendant que l'hôte
transfère et que le GPU compose, le fil du bureau dort et le cœur sert les
applications.

## 3. Où est le code

| Pièce | Fichier | Rôle |
|---|---|---|
| Géométrie, commandes 3D, convention, autotest (pur) | `src/drivers/display/composition_gpu.rs` | `soustrait`, `entrees_memoire`, `cree_texture` (drapeau `Y_0_TOP`), `transfert`, `encode_blit`, `vers_hote`, `juge_autotest` ; 13 tests hôte (`tools/display/test_composition_gpu.rs`) |
| Pilote | `src/drivers/display/virtio_gpu.rs` | négociation VIRGL (si proposé), sonde 3D, **autotest de convention** (`BOUCHAUD_GPU_COMPOSITION_AUTOTEST_OK`), textures par surface (`texture_pour`, liste de pages par descripteur chaîné), `presente_trame_gpu`, `oublie_surface` |
| Quelles fenêtres, quels pixels | `src/gui/composition.rs` | plan de la trame tiré du plan de scène : tout calque dessiné après une fenêtre la recouvre ; routage des dommages clients (`route_degat`) ; relevé `[GPU-COMPOSITION]` |
| Bureau | `src/gui/window_manager.rs`, `src/gui/widgets.rs` | `ferme_trame` (fin de trame GPU ou CPU), `compose_client` ne recopie que hors des morceaux GPU, transitions GPU↔CPU, texture rendue à la fermeture d'une fenêtre |
| Client générique | `src/gui/window.rs` (`App::Application`), `src/gui/client.rs` | tout programme ring 3 s'ouvre dans une fenêtre standard (`BO_AUTOSTART_APP=/chemin` pour les bancs) ; plus d'écran « Ladybird » pour les autres applications |
| Banc | `tools/ci/run_gpu_composition.sh`, `tools/ci/analyse_gpu_composition.py`, `tools/userland/mire-gpu.c`, `.github/workflows/gpu-composition.yml` | mire au pixel, A/B DOOM, repli 2D ; TCG et KVM en CI |

Activation : **toujours compilé**, actif seulement si l'hôte propose VIRGL et que
la sonde 3D **et** l'autotest de BLIT réussissent. Sinon : virtio-gpu 2D, BGA ou
GOP, comme avant. `gpu-composition on|off` (shell) sert à l'A/B. Les options
`p10-virgl-experimental` et `p10-universal-present` sont conservées sans effet
pour les scripts existants (`run.ps1 -P10GPU`).

## 4. Le défaut d'orientation, compris et mesuré

La PR #281 affichait l'image retournée et la corrigeait en recopiant les
lignes à l'envers sur le CPU. Cause réelle, mesurée au pixel le 10/11 avec la
mire (`mire-gpu`, chaque pixel encode sa position) : **le BLIT de
virglrenderer travaille en coordonnées GL brutes** et ignore le drapeau
`Y_0_TOP`, alors que les ressources `Y_0_TOP` sont stockées à l'envers. Sans
conversion, la capture montrait `écran 249..648 ← surface 172..571`, exactement
ce que prédit ce modèle (test hôte `convention_brute_reproduit_la_capture_du_10_novembre`).

Correctif : textures créées `Y_0_TOP` (transfert sans retournement CPU), et
rectangles du BLIT convertis (`vers_hote`). La convention n'est **pas
supposée** : au démarrage, un BLIT réel entre deux textures 8×8 est relu par
`TRANSFER_FROM_HOST_3D` et jugé au pixel dans les deux conventions ; le
compositeur ne s'active que si l'une donne exactement la mire.

## 5. Preuves

### 5.1 Locales, TCG (QEMU 8.2.2, virglrenderer 1.0.0, llvmpipe, Xvfb), 10/11

`tools/ci/run_gpu_composition.sh` :

| Banc | Résultat |
|---|---|
| Autotest de convention | `BOUCHAUD_GPU_COMPOSITION_AUTOTEST_OK convention=Brut blit_gpu=8x3 relu_au_pixel=64/64` |
| Mire, capture de l'écran **de l'hôte** décodée | `MIRE_OK zone=640x400 exacts=254934 faux=0 blancs=1066` (carré mobile et curseur) — écran 71..470 ← surface 0..399 |
| A/B DOOM, même périphérique, `gpu-composition on/off` | `AB_CONVAINCANT` (trois passes : ÷15,7, ÷14,3, ÷15,2) : temps CPU de dessin du bureau **≈ 190 µs/trame contre ≈ 2 800 µs** ; pixels clients recopiés par le CPU **0,64 M contre ≈ 530 M par 30 s (−99,9 %)** ; cadence identique (≈ 2 095 trames/30 s) |
| Fenêtre cachée (DOOM sous la mire) | `OCCLUSION_OK` : pixels clients recopiés par le CPU **14 M contre 533 M / 30 s** ; les dommages d'un client sont filtrés par la zone OPAQUE des calques supérieurs (`route_degat`, plusieurs rectangles, jamais leur englobant) |
| Repli sans VIRGL (`virtio-gpu-pci`) | `REPLI_OK` : compositeur éteint, DOOM en composition CPU, aucun échec |
| Endurance locale, 2 fenêtres, SMP4, 200 s | aucun `GPU_COMPOSITION_ECHEC`, aucune panique, une texture, cadence stable |

Le temps de **présentation** par trame (≈ 5,8 ms GPU contre ≈ 5,0 ms CPU sous
TCG) mesure surtout le GL logiciel de l'hôte (llvmpipe) et les allers-retours
de commandes ; le fil du bureau y dort (interruption). Sur un hôte avec un vrai
GPU, cette part revient au GPU ; elle n'est pas mesurée ici.

### 5.2 CI

Workflow `GPU composition` (`.github/workflows/gpu-composition.yml`), matrice
TCG + KVM sur runner hébergé : voir [SESSION_2026-11-10.md](SESSION_2026-11-10.md)
pour les runs et verdicts au SHA exact.

## 6. Ce qui n'est PAS fait (et ne doit pas être annoncé)

- **Ladybird/Skia rastérise toujours sur le CPU** (dans son processus ring 3).
  Le GPU compose ses pixels, il ne les calcule pas. Un raster GPU exige un
  pilote Vulkan invité (Venus) et une uAPI DRM : non commencé.
- Les éléments du bureau (fond, cadres, dock) restent peints par le CPU ; seule
  leur partie abîmée est transférée.
- Pas de pilote AMD/Trigkey : la composition GPU n'existe que sous QEMU VIRGL.
  Sur la Trigkey, l'affichage reste le GOP.
- Le gestionnaire de fenêtres (composition, entrées) est toujours un fil
  noyau ; seule la politique `windowd` (#261) est en ring 3. Voir la feuille de
  route : l'étape suivante est de déplacer la **composition** derrière une
  interface de commandes vers un `compositord` ring 3 qui piloterait le même
  GPU.
