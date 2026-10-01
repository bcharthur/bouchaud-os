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
| `courant.rs` | identité CPU-local, handoff courant, continuation synchrone |
| `creation.rs` | Task::new, placement, register |
| `commutation.rs` | switch_context, install, trampolines |
| `comptabilite.rs` | accounting user/kernel/idle |
| `ordonnancement.rs` | pick, steal, schedule, boucle idle (AP et BSP) |
| `lifecycle.rs` | exit/run/reap/process tree |
| `blocage.rs` | WaitQueue park/wake, signaux |
| `preemption.rs` | préemption IRQ et ticks |
| `metriques.rs` | SMP/BKL/process metrics |
| `sommeil.rs` | deadlines, sleep, alarmes |
| `futex.rs` | implémentation futex historique |
| `diagnostic.rs` | table tasks + création process |
| `banc_continuation.rs` | banc `continuation-banc` (QEMU) |

V11C est structurel : `futex.rs` reste l'implémentation historique sous BKL.
Le passage vers un mécanisme natif Bouchaud à buckets/verrous locaux appartient
désormais à V12.

## Idle et continuation synchrone (BOUCHAUD_CONTINUATION_SYNCHRONE_V1)

`KERNEL_CTX[cpu]` est l'idle du coeur, et rien d'autre : `secondary_cpu_loop`
sur un AP, une boucle dédiée amorcée par `assure_idle_coeur_zero` sur le BSP.
`run` / `run_noyau` garent la pile d'amorçage dans `CONTINUATION`, que seule la
fin de leur racine rend reprenable (`scheduler/continuation.rs`). Voir
`docs/CONTINUATION_SYNCHRONE.md`.

## Cycle de vie d'une tâche (BOUCHAUD_CYCLE_DE_VIE_V1)

L'état ne s'écrit que par les transitions nommées de `EtatAtomique`
(`endort`, `reveille`, `tue_parquee`, `meurt`) : des CAS, `Zombie` absorbant.
Une mort imposée à une autre tâche passe par `marque_zombie` → `condamne`
(`comptabilite.rs`) : tuée sur place si elle est parquée dans une attente
interruptible, réveillée si elle est bloquée sur son cœur, sinon elle meurt à
sa prochaine frontière (retour d'appel système ou de faute, préemption depuis
l'espace utilisateur, premier passage en espace utilisateur). Règle pure :
`scheduler/cycle_vie.rs`. Voir `docs/CYCLE_DE_VIE_TACHE.md`.
