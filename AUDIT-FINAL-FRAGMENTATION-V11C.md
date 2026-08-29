# Audit final de fragmentation — fin du cycle V11

## Fragmenté fortement

- BKL : état, attente, acquisition, métriques, diagnostic
- BKL handoff
- BKL scheduler suspend/resume
- acquisition BKL fine
- WaitQueue / WaitSource
- ReadinessSource par objet
- frontière `kernel/native`
- process/thread : mémoire, processus, Task, switch, accounting, scheduler,
  lifecycle, blocking, preemption, metrics, sleep, futex, diagnostic
- CPU idle
- IDT scheduler + exceptions + périphériques
- souris PS/2
- persistance
- Performance Observatory / flight recorder
- desktop scoped-BKL
- GUI déjà découpée en scene/damage/windowing/theme/etc.

## Encore monolithique ou seulement partiellement découpé

Ces domaines ne sont pas oubliés : ils deviennent les cibles **V12+**, car les
découper utilement implique maintenant de déplacer de la sémantique vers les
interfaces natives créées par V11B, pas seulement de déplacer des lignes.

1. `compat/linux/mod.rs`
   - doit devenir un dispatcher mince et des adaptateurs par famille ;
   - aucune logique native ne doit y être ajoutée.

2. poll/select/epoll
   - migration vers `kernel::readiness::ReadinessSource` par objet ;
   - registration/recheck/wake par source.

3. futex
   - `thread/futex.rs` est enfin isolé ;
   - prochaine étape : primitive native d'attente sur mot, buckets et verrous
     locaux, puis adaptateur Linux.

4. réseau
   - state/RX/TX/wait/readiness ;
   - suppression progressive des boucles d'interrogation.

5. `gui/window_manager.rs`
   - input/focus/z-order/drag-resize/composition/clients.

6. mémoire
   - le demand-fault est désormais isolé dans `thread/faute_memoire.rs` ;
   - pourra ensuite sortir vers un sous-système memory/fault natif.

7. polices
   - parsing/cache/rasterisation à sortir des longues sections noyau.

## Règle pour V12

V11 a construit les frontières.
V12 doit maintenant **réduire les dépendances globales** derrière ces
frontières, avec une modification fonctionnelle mesurable à la fois.
