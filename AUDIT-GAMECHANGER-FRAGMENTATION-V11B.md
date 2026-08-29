# Audit — fragments game changer après V11B

## Fortement fragmenté / frontières stables

- BKL et acquisition
- ordonnanceur BKL suspend/resume
- BKL handoff
- CPU idle
- préemption/IRQ timer
- souris PS/2
- persistance
- Performance Observatory
- desktop BKL scoped
- WaitSource natif
- ReadinessSource natif
- frontière `kernel/native`

## Restent les plus gros game changers

### P0 — process/thread.rs
Toujours le monolithe principal. À éclater sans changement sémantique en premier.

### P0 — futex
À sortir de `thread.rs` après sa fragmentation, puis à remplacer la table globale
par buckets/verrous locaux. Linux `SYS_futex` deviendra seulement un adaptateur.

### P0 — poll/readiness
Migrer objet par objet sur `ReadinessSource`. C'est probablement le meilleur
levier de réduction de contention après futex.

### P1 — réseau
Séparer state/RX/TX/wait/readiness pour éviter que `recvmsg` traverse des
structures globales.

### P1 — window_manager
Séparer input/focus/z-order/drag-resize/composition/client IPC.

### P1 — mémoire/page faults
Séparer registry/backing/cache/readahead/fault completion.

### P1 — compat/linux
Le gros `mod.rs` doit finir comme dispatcher fin + modules de traduction.
Aucune logique noyau native ne doit y vivre.

### P2 — polices
Séparer parsing/cache/rasterisation et faire le travail lourd hors BKL.
