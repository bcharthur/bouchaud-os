# Bouchaud OS — Preempt IRQ V8

## Ce que le V7 a prouvé

Le chemin souris est désormais sain : `entries == eoi == exit`, les paquets
progressent et le bottom-half vide les invalidations. Le freeze immédiat au
premier mouvement de souris n'est plus la cause principale.

Le dernier run a ensuite montré des stalls BKL avec `live_site=41`. Dans
`thread.rs`, 41 désigne le chemin `preempt_from_irq`.

Il existe toutefois une ambiguïté importante dans le code actuel :
`preempt_from_irq()` pose `stall_site=41` après avoir obtenu le BKL, puis possède
plusieurs retours anticipés. Le `KernelGuard` est bien détruit par Rust à ces
retours, **mais le site diagnostic n'est pas toujours effacé**. Un futur BKL
peut donc être accusé à tort du site 41.

V8 ferme cette ambiguïté et fait en même temps un test P0 fort sur CPU0.

## Fragmentation

```text
src/arch/x86_64/
├── idt.rs
└── idt/
    ├── preemption.rs
    ├── timer.rs
    └── reschedule.rs
```

Les trois fragments sont injectés avec `include!` dans le même module `idt`.

## Changement de comportement P0

```rust
pub const BSP_DEFER_DIRECT_IRQ_PREEMPT_V8: bool = true;
```

Sur CPU0/BSP uniquement, une préemption demandée depuis PIT ou reschedule IPI
ne fait plus immédiatement :

```text
hard IRQ -> preempt_from_irq() -> BKL -> context switch
```

Elle fait :

```text
hard IRQ -> request_deferred_preempt() -> EOI/retour
```

La préemption est consommée au prochain point sûr déjà existant, notamment à la
sortie d'un syscall via `take_need_resched()`.

CPU1-3 conservent le chemin de préemption IRQ direct.

C'est volontairement un mode diagnostic/mitigation. Une tâche utilisateur
purement CPU-bound placée sur CPU0 et ne faisant jamais de syscall peut avoir
une tranche trop longue. Ce mode sert à confirmer ou écarter le P0 BSP.

## Correction de diagnostic site 41

Pour les préemptions directes conservées sur les AP, V8 appelle
`stall_site_clear()` à la frontière IDT **après chaque retour** de
`preempt_from_irq()`. Les retours anticipés ne peuvent donc plus laisser 41
contaminer une acquisition BKL ultérieure.

## Nouveaux logs

```text
[PREEMPT-IRQ]
[PREEMPT-CPU]
```

Le rapport indique :
- demandes de préemption IRQ ;
- appels directs / retours ;
- demandes BSP différées ;
- nombre de clears du site ;
- continuations IRQ encore actives ;
- provenance BKL observée au moment du rapport.

Attention : `continuation_max_ns` n'est PAS une durée de tenue BKL. Un
`preempt_from_irq()` peut faire un context switch et ne revenir que lorsque la
pile IRQ sortante est replanifiée.

## Test

Appliquer après V7, puis :

```powershell
git diff --check

.\run.ps1 -Ladybird -LadybirdUrl "https://www.google.com/" 2>&1 |
    Tee-Object -FilePath preempt-v8-google.log
```

Tester d'abord le bureau 30-60 s avec souris/clavier, puis Ladybird/Google.

Analyse :

```powershell
python .\tools\perf\analyse-preempt-v8.py .\preempt-v8-google.log
```

## Interprétation

1. Plus de freeze + plus de `site41` long sur CPU0 :
   le chemin de préemption IRQ directe du BSP est très fortement impliqué.

2. Freeze persistant, mais `site41` disparaît :
   le site 41 V7 était surtout une provenance stale ; chercher le nouveau site.

3. `site41` >= 1 s persiste :
   il vient alors d'une préemption directe encore active (normalement AP), ou
   d'un site task-layer qui reste à nettoyer.

## Fichiers non touchés

V8 ne remplace pas :
- le pilote souris V7 ;
- le BKL V5.1 ;
- `src/fs/persistance.rs` ;
- `src/gui/client.rs` ;
- `src/kernel/process/thread.rs`.

Ce dernier point est volontaire : V8 met une frontière sûre autour de
`preempt_from_irq()` avant de réécrire le scheduler lui-même.
