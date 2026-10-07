> **HISTORIQUE.** Relevé ou jalon daté, conservé pour mémoire : il n'est pas réévalué à chaque passe et ne décrit pas forcément le HEAD courant. État vivant : section 1 du [README](../README.md).

# Handoff Scheduler NG -- 1er octobre 2026

Branche `claude/ladybird-observability-performance`, HEAD `9f7c5cf8` (+ le
commit qui ajoute ce fichier). Rapport : `docs/SCHEDULER_NG.md`. Preuves :
`docs/preuves/scheduler-ng-gel-20261001/MANIFESTE.md`.

Derniers commits : `000a99ef` retrait du quantum noyau + preemption noyau
sure ; `25d978e2` sonde de gel + instrumentation registre + sonde hote ;
`9f7c5cf8` releves hors hard IRQ + ligne serie IRQ masquees (fin du gel).

Acquis sur le HEAD exact : compilation BIOS (57 avert.) et UEFI (58),
garde-fous 157/157, tests hote 104/104. Rien d'autre.

A refaire sur le HEAD exact :
1. CI : CI Fast, Integration, Reliability V3, ladybird-native-browser,
   Native IPC (dispatch manuel) ;
2. scheduler-ng-banc SMP1/4/8 (>= 5 demarrages chacun) avec
   `tools/ci/sonde-gel/` : 0 gel, invariants 0, `serie_reentrees=0`,
   `refus_lecteur`/`if_ouvert` ;
3. session-probe SMP1/4, continuation-banc SMP1/2/4/8,
   `tools/ci/run_os_primitives.sh`, `tools/ci/run_system_health.sh`.

Ouvert : imgA smp8-6 sans FIN ; borne du jeton serie sous A+B ; usb-hid
~732 ms (quantum retire : points cooperatifs a etudier) ; preemption ciblee
du compositeur (preexistant).

Interdits maintenus : pas de quantum noyau ; pas de Phase 2/EEVDF tant que
les validations ci-dessus ne sont pas vertes ; RTL8168 hors chantier.
