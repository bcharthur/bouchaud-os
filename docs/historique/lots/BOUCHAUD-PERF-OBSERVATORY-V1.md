# Bouchaud Performance Observatory V1 — drop-in

## But

Faire apparaître *où* part la latence lorsque Ladybird/Google rame, au lieu de
confondre BKL, page faults, BrowserHost, WebContent et GUI.

## Fonctionnalités

- `kernel::perf` fragmenté en 5 fichiers fonctionnels.
- Flight recorder atomique de 2048 événements, sans allocation/verrou.
- Dump automatique `[PERF-FR]` avant le relevé riche lors d'une panique.
- Corrélation Wheel -> prochaine FrameReady.
- Mesure `frame_gap_max` et `input_to_frame_max`.
- Watchdog >= 500 ms de silence client.
- Classification déterministe :
  - `kernel-bkl`
  - `memory-pagefault`
  - `browser-renderer`
  - `healthy`
- Hook des alertes BKL V4 vers le flight recorder.
- Coalescence des Wheel consécutifs non lus : le delta total est conservé,
  mais on évite d'empiler des centaines d'événements de scroll périmés.
- Analyseur Python hors-ligne pour anciens et nouveaux logs.

## Ce que ce V1 ne prétend pas mesurer

Les phases internes *upstream Ladybird* (`JavaScript`, style, layout, paint,
raster) ne sont pas accessibles depuis le noyau. Ce V1 isole déjà le domaine
`browser-renderer` par exclusion instrumentée. Instrumenter JS/style/layout
nécessitera ensuite de reconstruire l'artefact natif Ladybird avec des sondes
dans LibWeb/WebContent.

## Fichiers suivis remplacés

- `src/kernel/debug/perf.rs`
- `src/kernel/debug/panic.rs`
- `src/gui/client.rs`
- `src/kernel/sync/bkl.rs` (V4 conservé + hook perf)

Le patch NE contient PAS `src/fs/persistance.rs`, afin de ne pas écraser ton
patch P1 déjà présent.

## Nouveaux logs

`[PERF-BROWSER]` :
- frames/inputs de la période ;
- Wheel fusionnés ;
- silence ;
- max frame gap ;
- max input->frame ;
- page faults ;
- goulot classé.

`[PERF-WATCHDOG]` :
- seulement lorsque silence >= 500 ms ou anomalie mesurée.

`[PERF-FR]` :
- dernières transitions performance au moment d'un panic.

## Test

Après extraction à la racine :

```powershell
git status --short
git diff --check
.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" |
    Tee-Object -FilePath perf-observatory-google.log
```

Scroller pendant 30-60 secondes, même si ça rame.

Puis :

```powershell
python .\tools\perf\analyse-bouchaud-perf.py .\perf-observatory-google.log
```

## Critères utiles

- `input_to_frame_max_ms` : UX directement ressentie.
- `frame_gap_max_ms` : trous de rendu.
- `wheel_coalesced_delta` : travail d'input évité.
- `pf_delta` : pression Memory Fabric.
- `bottleneck=...` : domaine prioritaire à optimiser.
