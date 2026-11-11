# P10 — compositeur GPU des fenêtres (`BOUCHAUD_GPU_COMPOSITION_V2`)

> Mis à jour le 10 novembre 2026, branche `claude/c22-p13-p10-gpu-kernel-0y2ucg` (§5.1–5.2 au SHA `540fdfb` ; §5.3–5.4 ajoutés ensuite).
> **État : VERIFIED_AT_KVM (QEMU VIRGL, CI hébergée), Ladybird réel compris.**
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

### 5.2 CI, SHA `540fdfb` — run [42784689821](https://github.com/bcharthur/bouchaud-os/actions/runs/42784689821) : **3/3 jobs verts**

| Job | Verdicts |
|---|---|
| `gpu-composition / kvm` (KVM, llvmpipe hôte) | `MIRE_OK exacts=254934 faux=0` ; `AB_CONVAINCANT` dessin CPU **25,5 µs/trame contre 467,5 µs (÷18,3)**, pixels clients CPU 0,65 M contre 537 M / 30 s, cadence 2 100 contre 2 101 ; `OCCLUSION_OK` 12,2 M contre 537 M ; `REPLI_OK` |
| `gpu-composition / tcg` | `GPU_COMPOSITION_BANC_OK` (mêmes cinq verdicts) |
| `gpu-composition / Ladybird` (artefact canonique, provenance vérifiée) | `LADYBIRD_BROWSER_HOST_OK`, `LADYBIRD_CONVERGENCE_OK … surface=1` (mire de la page vérifiée sur l'écran X de l'hôte), autotest de convention OK, `BOUCHAUD_GPU_COMPOSITION_OK` ; sur 30 s : 110 trames GPU, **69,3 M pixels posés par le GPU**, 12,2 M montés par l'hôte, **0,34 M** seulement recopiés par le CPU |

Sous KVM, l'invité **dort** pendant la présentation (≈ 4,5 ms/trame d'attente par interruption, GL logiciel llvmpipe côté hôte) : c'est le cœur invité rendu aux applications. Avec un vrai GPU côté hôte, cette part descend avec lui.

### 5.3 Panne GPU puis reprise (`BOUCHAUD_GPU_COMPOSITION_REPRISE_V1`) — TCG local, CI à chaque push

Une trame GPU refusée sans casser la file (réponse d'erreur de l'hôte) ne
désactive plus le compositeur pour toute la session : les textures sont
rendues, le bureau repasse **dans la même trame** en composition processeur,
et le GPU est retenté 5 s plus tard (trois reprises au plus, puis échec
définitif ; une file cassée reste définitive). L'injection n'existe que dans
une image de banc (`--features banc-gpu-composition-panne`, refus de la 300ᵉ
trame) ; le workflow vérifie que la chaîne `GPU_COMPOSITION_INJECTION` est
**absente** de l'image livrée.

Banc `panne` (même mire au pixel) : `GPU_COMPOSITION_INJECTION refus_trame=300`
→ `GPU_COMPOSITION_ECHEC raison=injection-banc file_cassee=0 repli=composition-processeur reprise=dans-5s`
→ `BOUCHAUD_GPU_COMPOSITION_REPRISE tentative=1` → 1 154 trames GPU ensuite,
`MIRE_OK exacts=254934 faux=0` sur la capture prise **après** la reprise :
`PANNE_OK`.

### 5.4 Où passent les 4 à 8 ms de présentation sous VIRGL ? (`BOUCHAUD_GPU_ATTRIBUTION_V1`)

Chaque commande virtio-gpu est chronométrée (envoi → réponse) et rangée dans
un histogramme par type ; chaque trame aussi (dessin CPU, présentation). Les
relevés `[GPU-TRAMES]` et `[GPU-COMMANDES]` donnent p50/p95/p99 (bornes hautes
de classes de puissance de deux) et le maximum. Banc local TCG du 10/11, DOOM,
30 s, mêmes conditions :

| Bras | Présentation p50/p95/p99 | `RESOURCE_FLUSH` p50/p95/p99 | `TRANSFER_TO_HOST_2D` p50 | `TRANSFER_3D` / `SUBMIT_3D` p50 |
|---|---|---|---|---|
| VIRGL, composition GPU | ≤ 8 192 / 16 384 / 16 384 µs | ≤ 4 096 / 8 192 / 16 384 µs | ≤ 256 µs | ≤ 1 024 / ≤ 1 024 µs |
| VIRGL, composition CPU | ≤ 8 192 / 16 384 / 16 384 µs | ≤ 4 096 / 8 192 / 16 384 µs | ≤ 1 024 µs | — |
| virtio-gpu 2D **sans** VIRGL | ≤ 2 048 / 8 192 / 16 384 µs | ≤ 1 024 / 2 048 / 8 192 µs | ≤ 512 µs | — |

**Attribution** : l'écart « VIRGL 4,3 ms contre 2D 1,2 ms » (KVM, CI) vient du
`RESOURCE_FLUSH` **sous périphérique VIRGL, que la composition soit GPU ou
CPU** : avec `virtio-gpu-gl-pci`, QEMU présente le scanout par le GL de l'hôte
(fenêtre SDL `gl=on`), et sur les runners ce GL est **llvmpipe** (Mesa
logiciel). Ce n'est pas le compositeur : ses commandes propres
(`TRANSFER_3D`, `SUBMIT_3D`) restent sous 1 ms au p50, et le bras CPU sur le
même périphérique paie le même flush. Pendant ces attentes le fil du bureau
dort (réponse par interruption). Sur un hôte avec un vrai GPU, ce flush est
le travail d'un GPU matériel : **non mesuré dans cette session (aucun GPU
matériel disponible)**.

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

## 7. Rastérisation GPU de Ladybird/Skia (étape C) — **BLOCKED_EXTERNAL**, première interface bloquante identifiée

Constat vérifié le 10/11, sans simuler de GPU :

1. **Ladybird n'a qu'un chemin GPU sous Linux : Vulkan** (Skia Ganesh/Graphite
   Vulkan ; pas de backend GL pour la peinture) — [audit P10](../ladybird/P10_GPU_AUDIT.md) §2,
   `SkiaBackendContext::create_independent_gpu_backend()`.
2. Dans l'invité, Vulkan accéléré sous QEMU = **Venus** (Mesa `vulkan_virtio`)
   sur virtio-gpu, qui exige du périphérique : `context_init` (capset Venus = 4),
   ressources **blob** (`RESOURCE_CREATE_BLOB`, `RESOURCE_MAP_BLOB`) et une
   fenêtre de mémoire hôte partagée (`hostmem`).
3. **QEMU 8.2.2 (cet hôte et les runners Ubuntu 24.04) n'expose aucune de ces
   propriétés** : `qemu-system-x86_64 -device virtio-gpu-gl-pci,help` ne liste
   ni `venus`, ni `blob`, ni `context_init`, ni `hostmem` (Venus est apparu
   dans QEMU 9.2). La preuve est donc impossible ici, quel que soit le code de
   l'invité : **BLOCKED_EXTERNAL (hôte QEMU ≥ 9.2 avec virglrenderer compilé
   `-Dvenus=true`)**.
4. Côté invité, **première interface manquante** : le nœud de rendu DRM
   `/dev/dri/renderD128` et l'uAPI `virtgpu` (`DRM_IOCTL_VIRTGPU_GETPARAM`,
   `GET_CAPS`, `CONTEXT_INIT`, `RESOURCE_CREATE_BLOB`, `MAP`, `EXECBUFFER`,
   `WAIT`), puis l'export dma-buf de l'image du Compositor vers la surface de
   fenêtre. Ni l'une ni l'autre n'existe dans `src/compat/linux` aujourd'hui.

Le plus grand palier **réel** atteignable sans cela est fait : la surface que
Skia rastérise sur le CPU (ring 3) est composée **sans copie CPU** par le GPU
(texture adossée à ses pages, transfert du seul rectangle abîmé). Lancer
Ladybird sans `--force-cpu-painting` avec un lavapipe invité exercerait le
code GPU sans rien accélérer : refusé (ce serait une simulation).

Ordre des travaux quand un hôte Venus existe : (a) uAPI `virtgpu` minimale sur
le pilote existant (contexte, blob, execbuffer, fence) avec test QEMU ;
(b) Mesa Venus statique dans l'image Ladybird ; (c) dma-buf de l'image du
Compositor → texture du compositeur (zéro copie) ; (d) A/B `--force-cpu-painting`
on/off sur le banc de performance Ladybird.

## 8. Trigkey (GPU AMD réel, étape D) — architecture, aucun essai PHYSICAL dans cette session

La composition est déjà séparée en deux couches : `src/gui/composition.rs`
(quelles fenêtres, quels rectangles : indépendant du matériel) et un pilote
qui sait (1) adosser une texture aux pages d'une surface, (2) monter un
rectangle abîmé, (3) poser des rectangles dans le scanout, (4) dire si une
trame a échoué. Pour un GPU AMD intégré (Trigkey), le pilote équivalent est :

| Fonction du compositeur | virtio-gpu VIRGL (fait) | AMD (à faire) |
|---|---|---|
| Énumérer, sonder | PCI 1AF4, capset VIRGL, autotest de convention | PCI 1002, lecture de l'IP discovery, firmware (PSP/SMU) chargés par le pilote |
| Texture adossée aux pages | `RESOURCE_CREATE_3D` + `ATTACH_BACKING` | table GART/VM du GPU pointant sur les pages de la surface |
| Monter le rectangle abîmé | `TRANSFER_TO_HOST_3D` | inutile : le GPU lit la mémoire système directement (cohérence à garantir) |
| Poser les rectangles | `SUBMIT_3D` de `VIRGL_CCMD_BLIT` | moteur **SDMA** (copie de rectangles, pas de shaders) ou plans matériels du DCN |
| Présenter | `RESOURCE_FLUSH` | flip du plan primaire DCN (sans déchirure, au vblank) |
| Échec, repli | `GPU_COMPOSITION_ECHEC` → CPU, reprise | même contrat : délai de garde, réinitialisation de l'anneau, repli GOP |

**Procédure sûre** (rien n'écrit sur le disque interne) : démarrage USB de
l'image Trigkey (`tools/release/build_trigkey_image.sh`), GOP conservé comme
repli, pilote AMD en lecture seule d'abord (identification, IP discovery,
journal série), puis SDMA sur une surface de test avec relecture au pixel
(même mire que `mire-gpu`), puis seulement le flip DCN. Chaque étape a un
interrupteur de démarrage et la composition CPU reste le chemin par défaut.
Statut : **NOT_TESTED (PHYSICAL)** — aucune machine dans cette session.

