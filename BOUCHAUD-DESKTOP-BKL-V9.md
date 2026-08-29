# Bouchaud OS — Desktop BKL Scoped V9

## But

V8 a montré que le freeze restant n'est pas expliqué par :
- IRQ12 souris ;
- HLT BSP ;
- préemption IRQ directe du BSP ;
- `site=41`.

Le pattern restant est un `task=0 / desktop` qui peut conserver le BKL pendant
plusieurs secondes.

Le contrat historique du noyau est volontairement conservateur :
`kernel_task_trampoline()` donne un KernelGuard racine au thread noyau et
`schedule()` suspend/restaure sa profondeur.

V9 ne casse pas ce contrat pour TOUS les kernel threads. Il introduit un mode
**Scoped BKL uniquement pour le desktop**, par safe points explicites.

## Sécurité du safe point

Un handoff n'est autorisé que si :

```text
current_is_kernel_task == true
task name              == desktop
IF                     == 1
BKL held               == true
BKL depth              == 1
```

`depth > 1` est systématiquement refusé. V9 ne coupe donc jamais une section
critique imbriquée ouverte par une fonction noyau.

## Fragmentation

```text
src/gui/
├── desktop_bkl.rs
├── desktop_bkl/
│   ├── etat.rs
│   ├── scope.rs
│   └── diagnostic.rs
├── reveil_v9.rs
└── framebuffer_v9.rs
```

`gui/mod.rs` redirige seulement `reveil` et `framebuffer` vers les wrappers V9.

Les fichiers historiques restent présents et inchangés :

```text
src/gui/reveil.rs
src/gui/framebuffer.rs
```

Les wrappers les incluent sous un module `legacy`, puis ne remplacent que les
frontières voulues.

## Safe points V9

### `reveil::note_tour`
Handoff au début du prochain tour GUI, avant lecture du nouvel état.

### `reveil::note_trame`
Handoff après une trame terminée.

### `reveil::note_trame_differee`
Empêche une boucle qui rate continuellement son slot de trame de garder le BKL.

### `framebuffer::present` / `present_rect`
La copie backbuffer -> LFB est exécutée **hors BKL** lorsque depth=1.

### `reveil::publie`
Le rapport périodique lit des atomiques et écrit le port série : il est exécuté
hors BKL afin que les logs eux-mêmes ne gonflent pas les temps de tenue.

## Handoff

Le mécanisme utilisé est le contrat existant :

```text
suspend_for_schedule()
    -> profondeur locale 0
    -> OWNER libéré
    -> wake des waiters BKL

petite fenêtre PAUSE

resume_after_schedule(depth=1)
```

La profondeur du KernelGuard racine est donc exactement restaurée avant de
retourner dans le code legacy.

Avec contention déjà visible (`parked` ou `resume`), la fenêtre utilise plus de
`PAUSE` afin que le wake ciblé ait le temps de faire avancer un AP sous TCG.

## Logs

```text
[KTHREAD-BKL]
[KTHREAD-BKL-SKIP]
[KTHREAD-BKL-SITE]
```

Les champs importants :

```text
mode=scoped
checkpoints=
scopes=
releases=
contended=
gap_current_ns=
gap_max_ns=
unlocked_ns=
reacquire_max_ns=
release_window_max_ns=
```

Le compteur `nested` dans `[KTHREAD-BKL-SKIP]` doit monter : c'est normal et
souhaitable. Cela prouve que V9 refuse de couper les profondeurs > 1.

## Test

V9 est incrémental sur V8/V7/V6/V5.1.

```powershell
git diff --check

.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" |
    Tee-Object -FilePath desktop-bkl-v9-google.log
```

Tester :
1. bureau + souris/clavier 30 s ;
2. lancer Ladybird ;
3. Google + scroll pendant 2 à 5 minutes.

Puis :

```powershell
python .\tools\perf\analyse-kthread-bkl-v9.py .\desktop-bkl-v9-google.log
```

## Critère P0

Le test est bon si :
- `releases` monte continuellement ;
- les `present*` accumulent du `unlocked_ns` ;
- les holds BKL `task=0` ne repartent plus à plusieurs secondes ;
- souris/clavier restent vivants ;
- les AP continuent de progresser.

Si un hold > 1 s persiste, son moment sera comparé à `gap_current_ns` et aux
compteurs de sites. On saura alors quelle phase du tour GUI ne traverse encore
aucun safe point.

## Limite volontaire

V9 n'est PAS encore la suppression globale du BKL des kernel threads.

La prochaine étape, si V9 est stable, sera de transformer les sous-systèmes GUI
partagés en verrous locaux, puis de supprimer le KernelGuard racine du desktop.
Faire cette dernière étape directement maintenant obligerait à auditer en une
fois chaque accès kernel implicite du window manager, ce qui serait beaucoup
plus risqué.
