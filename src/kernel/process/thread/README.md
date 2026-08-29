# `kernel/process/thread/` — carte V11C

`thread.rs` n'est plus un monolithe. Les fichiers ci-dessous sont injectés avec
`include!()` dans le même module Rust `kernel::task`.

| Fragment | Responsabilité |
|---|---|
| `modeles.rs` | constantes, priorités, états, contexte |
| `faute_memoire.rs` | demand paging / registry de fautes |
| `processus.rs` | Mm, Process, FileTable, mappings |
| `tache.rs` | structure Task |
| `etat_global.rs` | tables et atomiques scheduler |
| `diagnostic_stall.rs` | SMP-STALL, poll/VM probes |
| `courant.rs` | identité CPU-local, handoff courant |
| `creation.rs` | Task::new, placement, register |
| `commutation.rs` | switch_context, install, trampolines |
| `comptabilite.rs` | accounting user/kernel/idle |
| `ordonnancement.rs` | pick, steal, schedule, AP idle |
| `lifecycle.rs` | exit/run/reap/process tree |
| `blocage.rs` | WaitQueue park/wake, signaux |
| `preemption.rs` | préemption IRQ et ticks |
| `metriques.rs` | SMP/BKL/process metrics |
| `sommeil.rs` | deadlines, sleep, alarmes |
| `futex.rs` | implémentation futex historique |
| `diagnostic.rs` | table tasks + création process |

V11C est structurel : `futex.rs` reste l'implémentation historique sous BKL.
Le passage vers un mécanisme natif Bouchaud à buckets/verrous locaux appartient
désormais à V12.
