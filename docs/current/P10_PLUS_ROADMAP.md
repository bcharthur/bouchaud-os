# SUPER P10+ — de la présentation virtio-gpu 2D vers une pipeline accélérable

> Ce document distingue **présentation** (copier des pixels rendus par le
> processeur vers un écran) et **accélération** (faire calculer les pixels par
> un GPU). À ce jour, Bouchaud OS ne fait QUE de la présentation : aucun
> contexte 3D, aucune commande VIRGL/Venus, aucun rendu par le GPU. Rien ici
> n'autorise à annoncer WebGL, WebGPU ou Skia accélérés.
>
> États : NOT_STARTED · EXPERIMENTAL · IMPLEMENTED_LOCAL (code + tests hôte +
> QEMU local, sans preuve CI) · VERIFIED_AT_TCG · VERIFIED_AT_KVM ·
> PHYSICAL_PENDING · BLOCKED. Mis à jour le 2026-10-15 (campagne `b6c99032` lue).

## Architecture actuelle (vérifiée dans le code)

```
bureau (run_noyau, tâche noyau, cœur 0)        Ladybird (Compositor, WebContent)
        │ dessine dans BACK (Vec<u32>, CPU)              │ trames via le WM
        ▼                                                 ▼
  gfx::present_rect(x,y,l,h)  ── copie CPU ──►  mémoire DMA du scanout (1 bloc)
        │
        ▼  virtio_gpu::presente (SleepMutex GPU)
  TRANSFER_TO_HOST_2D (barrière, réponse jugée)  ─►  QEMU : ressource hôte
  RESOURCE_FLUSH      (barrière, réponse jugée)  ─►  QEMU : surface affichée
```

Une seule commande en vol (`VIRTIO_GPU_FLAG_FENCE` sur chacune, réponse jugée
par `virtio_gpu_file::juge`), mémoire DMA rendue seulement si le périphérique
ne peut plus la tenir (`virtio_gpu_dma`), repli BGA/GOP (`politique_scanout`).

## Jalons

| Jalon | État | Preuve | Reste à faire |
|---|---|---|---|
| **P10+.0** baseline sûre et mesurée | **VERIFIED_AT_KVM** (QEMU) sur `c7018e56` | os-primitives 38645396730 : transport, mire au pixel 3 721/3 721, bureau = BGA 921 375/921 600, 7 refus, `ATTACH_BACKING` sans réponse (mémoire conservée), réponse étrangère (file cassée), BGA/virtio TCG+KVM 640/640 ; KVM : plein 399 µs (p95 472), bande 102 (121), tuile 43 (62) | rejouer sur le HEAD qui porte P10+.1 ; Ladybird réel sur virtio (smoke diagnostic vert, 202 présentations) |
| **P10+.1** achèvement par interruption | **VERIFIED_AT_KVM (QEMU), reproduit** (`f23a1247`, `77e0e5de`) ; politique par défaut : **interruption** | A/B KVM CONVAINCANT deux fois (inactif plein 201,5 puis 136 ‰ contre 0 ; 0 achèvement perdu sur 3 840) ; pannes TCG+KVM (injection déterministe `9519547b`) | rejouée à chaque campagne |
| **P10+.2** double tampon, copies réduites | **VERIFIED_AT_KVM (QEMU), reproduit** (`77e0e5de`, `b6c99032` : plein 176 → 113,5 µs) ; politique par défaut : **composition directe** (`2a7ce3ed`) | [38704459647](https://github.com/bcharthur/bouchaud-os/actions/runs/38704459647) job gpu-tampon : pixels TCG 918 494 / KVM 921 336 sur 921 600, 0 octet copié, 0 accès pendant un transfert ; A/B KVM **CONVAINCANT** 6/6 (p50 plein 428 → 232,5 µs, bande 116 → 110, tuile 72 → 70), TCG CONVAINCANT ; pannes vertes en local avec ce défaut | reproduire l'A/B KVM ; double tampon à deux ressources seulement si une soumission asynchrone le justifie |
| **P10+.3b** une trame, une soumission | **VERIFIED_AT_KVM (QEMU), une campagne** ; défaut **on** (`ada19b78`) | [38732049308](https://github.com/bcharthur/bouchaud-os/actions/runs/38732049308) job gpu-tampon : A/B KVM CONVAINCANT 6/6 (16 → 5 commandes, p50 474 → 177,5 µs, p95 550 → 237,5) ; pixels TCG 921 272 / KVM 921 362 | reproduire l'A/B KVM ; un NON_CONVAINCANT rend le défaut à off |
| **P10+.3c/d** surface client à deux tampons (WM ↔ chrome Ladybird) | **GELÉ** pendant la stabilisation ; OFF par défaut (`617b5383`, `d2b80a9a`, `d378ce42`) | local TCG SMP2 : banc de déchirure CONVAINCANT (v1 51–75 compositions mixtes, v2 0) ; tests hôte et clang | banc KVM (non bloquant) ; smoke Ladybird réel sous `gui-surface double` ; aucune extension avant la fin de la stabilisation |
| **P10+.4** capacités 3D (virgl/venus) en bac à sable | NOT_STARTED | QEMU propose `0x0000010130000002` : VERSION_1, et des bits que le pilote **refuse** (aucune 3D négociée) | audit des QEMU des runners (rutabaga/virgl disponibles ?), preuve de raster GPU d'une primitive, OFF par défaut |
| **P10+.5** WebGL/Skia accéléré | NOT_STARTED, **BLOCKED** par P10+.4 | — | contrat CPU/GPU avec `HeadlessWebView`, processus GPU, perte de périphérique |
| **P10+.6** GPU physique Trigkey/AMD | PHYSICAL_PENDING (documentation seule) | — | aucune écriture disque, aucun flash, aucun démarrage physique sans Arthur |

## P10+.1 — ce qui est écrit

- `src/drivers/display/virtio_gpu_irq.rs` (pur, `tools/display/test_virtio_gpu_irq.rs`, 12 tests) :
  message MSI vers un LAPIC (`None` au-delà de l'APIC 255 : pas de remappage),
  emplacement de la table, `strategie(cedable, armee, voulue)`, et
  `attend()` : rotation 20 µs, puis sommeil jusqu'à l'échéance ou 2 ms
  (relecture de secours d'une interruption perdue, comptée), ticket pris
  **avant** la dernière relecture.
- `virtio_gpu.rs` : armement MSI-X (entrée 0 vers le LAPIC du cœur de la
  sonde, `queue_msix_vector` relu, refus → MSI-X désactivé, scrutation) ;
  gestionnaire `interruption()` sans verrou ni impression ; attente dormante
  seulement si `cession_volontaire_sure()` (tâche, IF=1, aucun verrou
  tournant) ; sinon la scrutation bornée d'avant (amorçage, contextes
  masqués).
- Banc : `banc-present tache` (contexte du bureau), `gpu-irq on|off`,
  `tools/ci/run_banc_irq_gpu.sh` + `analyse_banc_irq_gpu.py` (critères
  déclarés : sûreté, part inactive du cœur ≥ +100 ‰ en plein écran, p50 ≤
  1,15×, p95 ≤ 1,30×, relecture ≤ 1 %, quorum 4/6).
- Pannes : option `banc-virtio-gpu-irq-pannes` (jamais dans `default`),
  `tools/ci/run_virtio_gpu_irq_pannes.sh`.
- Non testé : reset du périphérique, retrait à chaud (pas de QMP dans les
  bancs), plusieurs files en vol (une seule par conception).

## P10+.2 — ce qui est écrit

- `src/drivers/display/virtio_gpu_tampons.rs` (pur, `tools/display/test_virtio_gpu_tampons.rs`, 8 tests) :
  états `Composition → Transfert → Composition` (réponse, barrière comprise)
  ou `Inconnu` (sans réponse, absorbant) ; seconde soumission refusée ; mode
  `Direct` seulement voulu, mémoire suffisante, tampon non inconnu ;
  `a_rejouer` (âge d'un tampon, pour un double tampon) et `octets_copies`.
- `virtio_gpu::presente` tient la mémoire en `Transfert` autour du
  `TRANSFER_TO_HOST_2D` ; le flush ne part qu'après une réponse OK.
- `gfx` : `gpu-tampon direct|copie` (pris au prochain `enter()`), `back()`
  rend la mémoire du scanout en composition directe et **compte** tout accès
  pendant un transfert ; `present`/`present_rect` comptent les octets copiés
  et présentés sans copie ; l'abandon du scanout en quitte la mémoire (plus
  aucune écriture, en copie comme en direct).
- Pourquoi un seul tampon d'abord : en 2D l'hôte ne lit la mémoire invitée
  que pendant le transfert, et `presente` est synchrone ; deux ressources
  n'apportent rien tant que la soumission n'est pas asynchrone — et coûtent
  le rejeu des dommages (une copie, comptée par `octets_copies`).
- Pas encore : soumission asynchrone (composer la trame suivante pendant le
  transfert), double tampon à deux ressources, Ladybird (le scanout virtio
  refuse le passage de l'écran au ring 3).

## Conception P10+.2 (historique de la conception)

Mesure qui motive : sous KVM la présentation plein écran coûte ≈ 400 µs, dont
la **copie CPU** de 3,7 Mo de `BACK` vers la mémoire DMA. Objectifs :

1. **Supprimer une copie** : dessiner directement dans la mémoire DMA du
   tampon arrière (deux ressources hôtes A/B, chacune attachée à son propre
   bloc DMA), le bureau compose dans le tampon inactif ;
2. **Bascule** : `SET_SCANOUT` sur la ressource prête (ou
   `RESOURCE_FLUSH` du rectangle endommagé seulement), la ressource
   précédente redevient le tampon arrière **après** la réponse clôturée du
   flush (sinon l'hôte peut encore la lire) ;
3. **Possession** : un état par tampon (`Composition` → `Transfert` →
   `Affiche` → `Libre`), transitions pures et testées sur l'hôte comme
   `virtio_gpu_dma` ; aucune réécriture d'un tampon que l'hôte peut lire ;
4. **Repli** : sans mémoire DMA suffisante (2 × 3,7 Mo) → simple tampon
   actuel ; panne d'une commande → abandon comme aujourd'hui ;
5. **Preuves** : copies par trame comptées (doit passer de 1 à 0 pour la
   composition), capture au pixel avant/après, test de déchirure (motif
   alterné, aucune trame mixte), pannes pendant la bascule, A/B KVM du coût
   et de la latence de trame.

Interdits rappelés : ne pas remplacer l'ordre virtio par l'hypothèse qu'une
sonnette ordonne deux commandes ; ne pas retirer la barrière pour gagner du
temps ; ne pas annoncer de « zéro copie » sans compter chaque copie.
