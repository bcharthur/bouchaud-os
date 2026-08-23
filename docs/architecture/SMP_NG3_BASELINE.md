# SMP-NG3 — baseline et audit de liveness

Baseline auditée : `97befb3` (`checkpoint: SMP-NG2 thread balancing with TLB deadlock repro`).

## Architecture observée

* `smp_lock` est un BKL réentrant. Il sérialise encore les syscalls, les fautes
  de page et plusieurs IRQ. Les transitions `OWNER`/`DEPTH` masquent brièvement
  les IRQ locales; l'attente active/`hlt` ne les masque pas lorsqu'elle vient du
  contexte tâche.
* Le scheduler conserve une table globale de tâches avec `runq_cpu`,
  `last_cpu`, `on_cpu` et `affinity_mask`. Les changements de contexte déposent
  puis reprennent explicitement le BKL avec `suspend_for_schedule` et
  `resume_after_schedule`.
* `AddressSpace` possède une PML4 et un masque atomique `active_cpus`, mais pas
  encore de verrou MM fin. Les pages et tables restent des `Vec` protégés de
  fait par le BKL.
* `munmap` retire les PTE, invalide localement, attend un shootdown distant, puis
  libère les frames. L'ordre PTE → ACK → free protège correctement contre le
  stale-TLB/UAF tant qu'un seul émetteur existe.
* Le shootdown NG2 emploie une mailbox globale et suppose que le BKL sérialise
  tous les émetteurs. Le handler IPI est atomique, sans allocation, sans BKL et
  ACK après `invlpg`/rechargement CR3.
* IRQ0/PIT à 1 kHz fournit à la fois ticks scheduler et temps monotone. Le BSP
  diffuse un IPI de reschedule tous les quatre ticks. Sous TCG, retarder IRQ0
  ralentit donc artificiellement tous les délais noyau.

## Cause confirmée du gel `munmap`

Le cycle est présent dans le code, même si le handler TLB lui-même ne prend pas
le BKL :

1. le dispatch de syscall détient le BKL; `sys_munmap` entre dans
   `AddressSpace::unmap`, publie la mailbox et attend tous les ACK;
2. une faute de page ou une IRQ classique sur un CPU cible entre par une porte
   d'interruption (IF effacé), puis appelle `smp_lock::enter`;
3. `wait_for_owner_change` conserve le spin lorsque IF était nul;
4. le CPU cible ne peut donc pas accepter l'IPI TLB, tandis que l'émetteur ne
   libère pas le BKL avant son ACK.

C'est un cycle BKL → ACK CPU et CPU → BKL. Augmenter un timeout ou abandonner
l'ACK serait incorrect : une frame pourrait être réutilisée avec une traduction
ancienne. La correction architecturale doit séparer préparation PTE et retraite
de frames, protéger l'espace par un verrou MM, quitter tous les verrous dont le
CPU cible peut avoir besoin avant l'attente, puis seulement libérer les frames.
La mailbox unique devra également être remplacée ou sérialisée avant le retrait
du BKL.

## Cartographie des chemins prioritaires

| Domaine | Emplacements principaux | Risque actuel |
|---|---|---|
| BKL/guards | `kernel/smp_lock.rs`, `kernel/task.rs`, `arch/x86_64/idt.rs`, `arch/x86_64/usermode.rs` | IRQ/exception bloquante avec IF=0; I/O et waits historiques sous BKL |
| Scheduler | `kernel/task.rs`, `arch/x86_64/cpu_local.rs`, `arch/x86_64/smp.rs` | table globale; PIT BSP et broadcast IPI; ownership logique plutôt que runqueue physique |
| MM/#PF | `kernel/vmm.rs`, `kernel/vma.rs`, `kernel/abi/mem.rs`, `arch/x86_64/idt.rs` | #PF sous BKL; aucune exclusion fine par espace/page; lecture file-backed sous sérialisation globale |
| TLB/CR3 | `arch/x86_64/smp.rs`, `kernel/vmm.rs` | attente synchrone sous BKL; mailbox globale; broadcast au lieu du masque actif |
| Temps/timers | `kernel/timer.rs`, `arch/x86_64/idt.rs`, `kernel/abi/mod.rs`, `kernel/abi/file.rs` | temps dérivé des ticks livrés; poll/nanosleep/timerfd/futex héritent du retard TCG |
| IPC/waits | `kernel/abi/file.rs`, `kernel/abi/futex.rs`, `kernel/task.rs` | boucles de sommeil/polling et reprises BKL; audit WaitQueue requis |
| Réseau | `net/`, `drivers/e1000.rs`, `kernel/abi/net.rs` | échéances PIT, polling smoltcp/socket et contention BKL |
| Disque | `drivers/ata.rs`, fautes file-backed dans `kernel/task.rs` | PIO/attente et lecture de page dans un chemin #PF globalement sérialisé |

## Ordre de correction

1. ajouter un verrou MM IRQ-safe et une retraite différée afin que le shootdown
   attende sans BKL ni emprunt `RefCell` vivant;
2. sérialiser ou mettre en file les requêtes concurrentes, cibler
   `active_cpus`, et instrumenter publication/IPI/ACK;
3. sortir le #PF du BKL, avec état Missing/Loading/Present/Failed et attente
   événementielle hors spinlock/I/O;
4. utiliser le TSC architectural pour les deadlines, puis installer un timer
   LAPIC local par CPU;
5. migrer vers de vraies runqueues et des WaitQueue avant de retirer le BKL des
   autres sous-systèmes.

## Invariants de validation

* aucun handler IPI TLB ne prend de lock dormant, le BKL ou un lock MM;
* aucune frame retirée n'est libérée avant les ACK de tous les CPU actifs;
* aucun émetteur n'attend avec IF=0 ou avec un verrou nécessaire au CPU cible;
* deux émetteurs ne peuvent écraser leurs paramètres/générations;
* le temps monotone progresse sans IRQ scheduler;
* aucun résultat QEMU/Ladybird n'est déclaré sans exécution réelle.
