# SUPER P10+ — de la présentation virtio-gpu 2D vers une pipeline accélérable

> Ce document distingue **présentation** (copier des pixels rendus par le
> processeur vers un écran) et **accélération** (faire calculer les pixels par
> un GPU). À ce jour, Bouchaud OS ne fait QUE de la présentation : aucun
> contexte 3D, aucune commande VIRGL/Venus, aucun rendu par le GPU. Rien ici
> n'autorise à annoncer WebGL, WebGPU ou Skia accélérés.
>
> États : NOT_STARTED · EXPERIMENTAL · IMPLEMENTED_LOCAL (code + tests hôte +
> QEMU local, sans preuve CI) · VERIFIED_AT_TCG · VERIFIED_AT_KVM ·
> PHYSICAL_PENDING · BLOCKED. Mis à jour le 2026-10-15.

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
| **P10+.1** achèvement par interruption | **IMPLEMENTED_LOCAL** (`daa5df34`) ; politique par défaut : **scrutation** | MSI-X armé (3 entrées, vecteur 0x50) ; A/B local TCG 6 tours : 0 cession sous interruption (médianes en scrutation 1 304 / 650 / 426), 0 interruption perdue, **NON_CONVAINCANT** (gain processeur plein écran 53 ‰ seulement, p95 bande/tuile dégradé) ; pannes TCG : 4 réponses rattrapées par la relecture (MSI-X masqué), 6 parasites sans progrès, attente aveugle échue à 1 s, abandon, bureau vivant | A/B **KVM** (CI) ; si CONVAINCANT, basculer le défaut (et le garde) avec la preuve ; sinon régler la rotation (20 µs) sur la mesure KVM, jamais sur TCG |
| **P10+.2** double tampon, copies réduites | NOT_STARTED | — | voir « Conception P10+.2 » |
| **P10+.3** interface Compositor/WM ↔ GPU | NOT_STARTED | — | contrat de surface typé, soumission des dommages sans capture, latence de trame p50/p95/p99 |
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

## Conception P10+.2 (à écrire, prochain incrément)

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
