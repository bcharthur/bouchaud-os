# Scheduler NG — rapport de chantier

Point de depart : `stable/continuation-green-20261001` (`9fcc13c5`), toute la
CI verte. Ligne de base : `docs/SCHEDULER_NG_BASELINE.md`. Banc :
`tools/userland/scheduler-ng-banc.c` + `tools/ci/scheduler_ng_baseline.py`.

Classes de preuve : **hote** (modeles exhaustifs, fils reels), **QEMU** (TCG,
4 coeurs hote ; SMP8/16 sur-souscrits), **CI**, **physique** (aucune a ce
stade : pas d'image physique avant la fin d'un jalon totalement vert).

## 1. Architecture avant / apres

```
AVANT                                   APRES (lots livres)
-----                                   -------------------
etat : store libre (range)              etat : 4 transitions CAS nommees,
  Zombie ecrasable par Blocked            Zombie absorbant (cycle_vie.rs)
mort imposee : Zombie ecrit sur         mort imposee : CONDAMNATION ; la tache
  une tache qui tourne                    meurt a sa frontiere, ou sur place
                                          si parquee en attente interruptible
exit_group / execve : sans attente      attendent la mort des freres
passation <-> reveil : Release/Acquire  barriere SeqCst des deux cotes
  (tache prete perdue sous x86-TSO)
descripteurs : fermes au recyclage      fermes a la mort, avant le parent
  de l'emplacement (EOF jamais)
sched_getaffinity : 1 en dur            masque reel de la tache
```

Inchange a ce stade : files par coeur a deux bandes (bitmaps sans verrou),
tourniquet au bit, vol de travail, quantum fixe de 4 ms, classes
interactive/normale, porte de transition par coeur, aucun verrou global.

## 2. Lots

| lot | commit | defaut | preuve |
|---|---|---|---|
| phase 0 — banc et ligne de base | `b8176797` | aucune mesure | 15 demarrages SMP1-16 |
| phase 1 — cycle de vie CAS | `6ec548bc` | 222-270 fils ressuscites | hote exhaustif ; QEMU 0 |
| passation/reveil ordonnes | `325992a6` | tache prete perdue (x86-TSO) | modele TSO + fils reels ; QEMU 0 orpheline |
| descripteurs a la mort | `d76bd7ee` | fin de fichier jamais livree | QEMU eof 10/10, racine 5/5 |
| affinite visible | `cd648655` | sysconf = 1 coeur | QEMU fils voit N |
| veille d'attente vive (obs.) | `0f06ebb8` | attentes longues sans coupable | attribution en direct |
| releves globaux espaces | (ce lot) | sortie de fil = 7-10 lignes serie, coeur tenu | episodes > 50 ms SMP4 : 59 -> 0-5 |

Details : `docs/CYCLE_DE_VIE_TACHE.md`.

## 3. Avant / apres (QEMU, mediane)

Rempli a chaque jalon depuis les campagnes `scheduler-ng-banc`.

### Attribution des longues attentes (phase 3)

`[SCHED-NG-ATTENTE-VIVE]` releve, PENDANT l'attente, toute tache prete depuis
plus de 50 ms et ce qui occupe son coeur. Sur scheduler-ng-banc SMP4 :

| cause | avant | apres espacement des releves globaux |
|---|---|---|
| tache dans `exit_group` ecrivant sur COM1 (`uart16550::write_lot`, jeton d'emission) | 59 episodes, 50-631 ms | 0 |
| fil noyau `usb-hid` (non preemptable hors demande ciblee) | — | 1 episode, jusqu'a 732 ms |
| releve `smpstat` du shell (hors tache) | — | fin de banc seulement |

Classe de preuve : QEMU. Le cout serie est propre a l'emulation d'un COM1 ;
la machine de reference n'a pas de port serie (`com1=bus-flottant`), mais la
tache qui meurt tenait aussi le verrou `lifecycle` pendant `CPU_CUMUL`.

## 4. Invariants

| invariant | etat | garde |
|---|---|---|
| zombie jamais remis Ready/Blocked | tenu par construction (CAS) | `verifie-cycle-vie.py` |
| fil d'un processus recolte jamais en espace utilisateur | 0 sur 52 demarrages | banc `ressuscites` |
| tache prete jamais hors de toute file | 0 `[SCHED-ORPHELINE]` sur 16 | `verifie-passation-reveil.py` |
| aucun reveil futex perdu | 0 | banc `reveils_perdus` |
| aucun BKL, aucun verrou global d'ordonnanceur | tenu | `verifie-ordonnanceur-sans-bkl.py`, `verifie-bkl-supprime.py` |

## 5. Ouvert

Voir `docs/SCHEDULER_NG_BASELINE.md` §4 (attentes de plusieurs centaines de
millisecondes, equite a 16 calculs, classe interactive sans effet sous
charge) et `docs/CYCLE_DE_VIE_TACHE.md` §5.
