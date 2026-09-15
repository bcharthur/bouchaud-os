BOUCHAUD OS — WEBCONTENT LIVENESS / RX IRQ V1
=================================================

Base cible
----------
Branche : perf/gui-event-driven
Base : 981363e9a00c321390868a4b592364baa7fc01b5

But
---
Ce patch attaque un défaut concret observé sur les sites lourds :

- sys_poll() force aujourd'hui un réveil toutes les 2 ms dès qu'une socket est présente,
  uniquement parce que le e1000 n'a pas de réveil RX matériel.
- Sur Google, le runtime observé monte à ~89 770 acquisitions poll sur une fenêtre
  de 5 s, avec ~4,44 s cumulées d'attente BKL.
- Le e1000 masque actuellement toutes ses interruptions et fonctionne en polling.

Le patch transforme le chemin en :
    paquet RX e1000
      -> IRQ11 QEMU
      -> acquittement ICR
      -> wake readiness
      -> poll se réveille immédiatement
      -> scan socket / drain réseau

Un watchdog de 50 ms reste volontairement présent quand l'IRQ RX est active.
Il borne une IRQ perdue et préserve un mécanisme de secours. Si la carte n'est
pas routée sur IRQ11, le fallback historique 2 ms est conservé automatiquement.

Fichiers modifiés
-----------------
- src/drivers/network/e1000.rs
- src/arch/x86_64/interrupts.rs
- src/arch/x86_64/idt.rs
- src/compat/linux/file.rs

Application
-----------
Depuis la racine du repo :

    .\APPLY-WEBCONTENT-LIVENESS.ps1
    .\VERIFY-WEBCONTENT-LIVENESS.ps1
    .\RUN-WEBCONTENT-STRESS.ps1

Le script ne fait ni commit, ni git clean, ni reset, ni add -A.
Il crée une sauvegarde des quatre fichiers avant modification.

Au boot, on veut voir :

    [kernel] e1000: RX interrupt-driven irq=11

Puis comparer poll à l'ancien ordre de grandeur :
    acq_total ~ 89 770 / 5 s
    wait_total_ns ~ 4,44 s / 5 s

Le but est une chute forte de ces deux valeurs et un Google qui reste interactif.

Important
---------
Ce ZIP cible un défaut source confirmé. Si WebContent reste à 100 % CPU alors que
la tempête poll a disparu, cela isolera proprement le problème restant vers
futex / boucle d'événements / production de frames Ladybird.
