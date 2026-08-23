# Télémétrie SMP comparable

Le Window Manager déclenche environ une fois par seconde un snapshot sous le BKL
déjà utilisé par le diagnostic historique. Aucun compteur chaud ne journalise:
les context switches, faults, migrations, steals et BKL ne font que modifier des
atomiques/entiers; la construction des chaînes n'arrive qu'au snapshot.

## Machine

Format stable, une seule ligne:

```text
[SMP-SAMPLE] v=1 t_ns=N window_ns=N load=[..] runnable=[..] rq=[..] ctx_delta=N mig_delta=N steal_ok_delta=[..] steal_try_delta=[..] steal_rej_bal_delta=[..] steal_rej_aff_delta=[..] bkl_wait_delta_ns=N bkl_hold_delta_ns=N bkl_acq_delta=N pf_delta=[..] tlb_delta=N
```

Toutes les listes ont `schedulable_cpus()` entrées. Les compteurs suffixés
`delta` couvrent exactement `window_ns`. `load` est l'utilisation physique par
CPU; `runnable` vient de l'état canonique des Tasks; `rq` expose la file physique.
Le parseur calcule moyenne, spread et déséquilibre sans supposer qu'un workload
mono-thread devrait remplir tous les CPU.

## Processus

```text
[PROC-SAMPLE] v=1 t_ns=N pid=N name=NAME cpu_pct=N cpu_map=[..] ctx_delta=N mig_delta=N runnable_threads=N threads=N rss=N vss=N
```

`cpu_pct=100` signifie un CPU logique; `cpu_map` est la contribution du processus
sur chaque CPU. RSS et VSS sont des octets. Les deltas de switches/migrations ont
la même fenêtre que le runtime processus.

## Outils host

`python tools/perf/analyze-smp-log.py run.log` produit les taux par seconde,
l'imbalance, le BKL, faults/TLB et un tableau processus. `compare-smp-logs.py`
affiche baseline, candidat et delta sans porter de jugement automatique.
