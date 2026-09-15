# Bouchaud OS — Event-Driven Core patch

Base attendue : Final V12 appliqué.

Ce patch applique les deux priorités P0 restantes :

1. **CPU0/BSP dort réellement avec `sti; hlt`** au lieu du mode diagnostique
   V6 qui faisait des PAUSE et revenait immédiatement au scheduler.
2. **Le desktop libère complètement son BKL racine pendant l'attente INTERFACE**.
   L'attente passe alors par WaitSource + WaitQueue V12 à depth=0.

## Pourquoi

V6 avait `BSP_SAFE_IDLE_DIAGNOSTIC=true` volontairement. Cette expérience est
terminée : la laisser active explique un BSP très chargé au repos.

Le desktop, lui, possède encore un KernelGuard racine historique. Pendant son
attente événementielle, V12 voyait donc depth>0 et gardait le chemin WaitQueue
legacy. Le patch fait désormais :

```text
desktop depth=1
 -> suspend_for_schedule() => depth=0
 -> WaitSource / WaitQueue detached
 -> vrai scheduler idle / HLT
 -> réveil événement/deadline
 -> resume_after_schedule(1) une seule fois
 -> prochain tour GUI
```

## Fragmentation nouvelle

```text
src/arch/x86_64/cpu/idle/politique.rs

src/kernel/sync/
├── reveil.rs
└── reveil/
    ├── types.rs
    ├── etat.rs
    ├── signal.rs
    ├── attente.rs
    ├── diagnostic.rs
    └── global.rs
```

Les wrappers GUI existants reçoivent seulement des sites de phase 700..773.

## Sites desktop

```text
700 tour
730 composition/culling
740 trame terminée
745 trame différée
750/751 present
752/753 present_rect
760 rapport
770 préparation wait
771 sommeil detached
772 reprise BKL
773 retour wait
```

## Test

```powershell
git diff --check

.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" |
    Tee-Object -FilePath event-driven-core.log

python .\tools\perf\analyse-event-driven-core.py .\event-driven-core.log
```

Cibles :
- `bsp_hlt=1`, `bsp_safe=0`;
- CPU0 `sched_wakes` / `wfi_wakes` progressent ;
- `safe_returns` CPU0 restent à 0 ;
- `INTERFACE-WAIT detached` monte ;
- `depth_violations=0`;
- forte baisse du CPU BSP au repos ;
- disparition ou forte réduction des holds BKL multi-secondes.
