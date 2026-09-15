BOUCHAUD OS - P0 #1 - TARGETED SCHEDULER IPI V1
===================================================

BUT
---
Sous QEMU TCG, le fallback scheduler utilise le PIT BSP. Le code actuel
broadcast un IPI de reschedule vers TOUS les AP a chaque quantum de 4 ms.

Dans le run de reference, les compteurs [SMP-IPI] des AP montent d'environ
+250/s chacun, y compris quand ils n'ont aucune tache utile a executer.

Ce patch conserve le quantum 4 ms mais remplace le broadcast par des IPI cibles
uniquement vers les AP qui executent actuellement une tache utilisateur.

Pourquoi c'est le P0 #1 :
- c'est un travail manifestement inutile dans le cas idle ;
- il est amplifie par TCG ;
- chaque IPI traverse le chemin scheduler/preemption et tente le BKL ;
- le correctif est petit, reversible et mesurable ;
- les reveils de nouvelles taches restent geres par reschedule_cpu(), donc on ne
  transforme pas ce P0 en refonte risquee du scheduler.

APPLICATION
-----------
Extraire ce ZIP a la racine du depot, puis :

  .\APPLY-P0-SCHED-IPI-TARGETED.ps1 -Preview
  .\APPLY-P0-SCHED-IPI-TARGETED.ps1
  .\VERIFY-P0-SCHED-IPI-TARGETED.ps1 -Build
  .\run.ps1

Le script modifie :
  src\kernel\process\thread.rs
  src\arch\x86_64\idt.rs

Un backup est cree automatiquement dans :
  .bouchaud-history\backups\.bouchaud-p0-targeted-sched-ipi-<timestamp>\

TEST A/B
--------
1. Demarrer Bouchaud OS.
2. Ne rien lancer pendant 8 a 10 secondes.
3. Relever deux lignes [SMP-IPI] espacees d'environ 5 secondes.
4. Avant le patch, c1/c2/c3 gagnaient typiquement ~1250 sur 5 secondes.
5. Apres le patch, les AP idle doivent rester presque plats.
6. Ouvrir Ladybird.
7. Attendre :
   BROWSER_HOST_INITIALIZED
   WEBCONTENT_READY
   M11_READY
   M11_GUI_HANDSHAKE_OK
   M11_DOCUMENT_LOADED page=1 url=https://example.com/
8. Verifier le rendu, souris, clavier et navigation.

METRIQUES A COMPARER
--------------------
- [SMP-IPI] : delta par CPU
- [BKL-STATS] : wait_ns, acquisitions, try_enter, max_hold_ns
- [SMP-SAMPLE] : bkl_wait_delta_ns, ctx_delta, deferred_preempt_delta
- temps entre PERF_EXEC_START / M11_READY / M11_DOCUMENT_LOADED

ATTENTION
---------
Ce P0 ne pretend PAS corriger le plus gros verrou structurel.

Les logs montrent aussi des execve (syscall 59) qui peuvent conserver le BKL
pendant plusieurs centaines de ms a plusieurs secondes. Ce sera le P0 suivant :
execve prepare/commit, apres avoir mesure ce patch isole.

ROLLBACK
--------
  .\ROLLBACK-P0-SCHED-IPI-TARGETED.ps1
