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
| releves globaux espaces | `cd6bc0a8` | sortie de fil = 7-10 lignes serie, coeur tenu | episodes > 50 ms SMP4 : 59 -> 0-5 |
| jeton serie : proprietaire, priorite IRQ, releve tenu une fois | `c24b11a2` | releve IRQ du minuteur : coeur zero tenu, interruptions masquees, 0,46 a 12,6 s | releves complets > 100 ms : 7 sur 129 (pire 12 622 ms) -> 2 sur 99 (pire 127 ms) |
| quantum des fils noyau | (ce lot) | fil noyau preemptable seulement sur demande ciblee : `usb-hid` 732 ms sur un AP | 1 a 10 preemptions au quantum par demarrage ; aucun fil noyau ne tient un coeur > 64 ms (15 demarrages) |

Details : `docs/CYCLE_DE_VIE_TACHE.md`.

## 3. Avant / apres (QEMU, mediane)

Rempli a chaque jalon depuis les campagnes `scheduler-ng-banc`.

### Attribution des longues attentes (phase 3)

`[SCHED-NG-ATTENTE-VIVE]` releve, PENDANT l'attente, toute tache prete depuis
plus de 50 ms et ce qui occupe son coeur. Sur scheduler-ng-banc SMP4 :

| cause | avant | apres espacement des releves globaux |
|---|---|---|
| tache dans `exit_group` ecrivant sur COM1 (`uart16550::write_lot`, jeton d'emission) | 59 episodes, 50-631 ms | 0 |
| fil noyau `usb-hid` (non preemptable hors demande ciblee) | — | 1 episode, jusqu'a 732 ms ; avec le quantum noyau : aucun fil noyau > 64 ms |
| releve `smpstat` du shell (hors tache) | — | fin de banc seulement |
| releve d'ordonnancement de l'IRQ du minuteur (`[SCHED-DUMP] raison=latence-hid`), interruptions masquees, qui disputait le jeton serie ligne par ligne | 463-566 ms (SMP1), 5,0-12,6 s (SMP4/8) | pire 127 ms |

Classe de preuve : QEMU. Le cout serie est propre a l'emulation d'un COM1 ;
la machine de reference n'a pas de port serie (`com1=bus-flottant`), mais la
tache qui meurt tenait aussi le verrou `lifecycle` pendant `CPU_CUMUL`.

### Le jeton d'emission serie (BOUCHAUD_JETON_SERIE_PROPRIETAIRE_V1)

Le releve complet que demande un pic de reveil s'imprime depuis l'IRQ du
minuteur du coeur zero, interruptions masquees (une vingtaine de lignes).
`_print` serialise chaque ligne par un jeton pris par CAS, attente bornee a
100 000 tours. Deux defauts :

1. la tache coupee par l'IRQ au milieu d'une ligne tenait le jeton sur le
   MEME coeur : elle ne pouvait le rendre qu'apres l'IRQ, et l'IRQ allait a
   la borne pour chacune de ses lignes (SMP1 : 463, 508, 566 ms ; les trois
   fois, la ligne `[SMP-SNAPSHOT]` est coupee au milieu d'une ligne de la
   tache) ;
2. contre les taches des autres coeurs, un CAS n'est pas equitable : l'IRQ
   perdait encore et encore, et allait a la borne ligne apres ligne
   (SMP4/SMP8 : 5,0 a 12,6 s).

Correction : le jeton porte son coeur (`cpu_index() + 1`) -- un ecrivain qui
le trouve tenu par son propre coeur ecrit sans attendre ; un ecrivain aux
interruptions masquees qui attend leve une priorite que les autres
respectent ; le releve prend le jeton une fois pour toutes ses lignes
(`tiens_emission`). `[SONDE-IRQ-DUREE]` publie `serie_imbriquees` et
`serie_bornes` (attentes allees a la borne). Garde `verifie-jeton-serie.py`
(6 negatifs).

Mesure (QEMU, scheduler-ng-banc, meme banc ; avant : 37 demarrages SMP1/4/8
sur `cd6bc0a8` et `cd6bc0a8` + quantum noyau ; apres : 30 demarrages SMP1/4/8,
avec et sans quantum noyau) :

| | avant | apres |
|---|---|---|
| releves complets de l'IRQ > 100 ms | 7 sur 129 (SMP1 463 ms ; SMP4/8 129 ms a 12,6 s) | 2 sur 99 (127, 118 ms) |
| echecs / perdus / ressuscites | 0 / 0 / 0 | 0 / 0 / 0 |

Les releves de plusieurs secondes d'avant peuvent inclure le gel decrit
ci-dessous, qui subsiste apres ; ceux de SMP1 sont le defaut 1 seul.

Classe de preuve : QEMU. Le jeton est pris meme sans COM1 (la machine de
reference ecrit dans le tambour RAM) : le defaut 1 existe aussi en physique,
avec une fenetre plus courte.

### Quantum des fils noyau (BOUCHAUD_QUANTUM_NOYAU_V1)

Un fil noyau n'etait preemptable que sur DEMANDE CIBLEE (reveil d'une tache
sensible a la latence). Un fil noyau qui travaille sans dormir tenait donc
son coeur : `usb-hid`, 732 ms sur un AP, `fork-exit` p99 1,26 s dans ce
demarrage. Desormais, au tic (BSP) ou a l'IPI de quantum (AP), un fil noyau
qui tourne depuis `QUANTUM_NOYAU_NS` (deux quanta, 8 ms) alors que des
taches attendent CE coeur est preempte, sous la meme condition de surete que
la demande ciblee (`preemption_noyau_sure` : aucun verrou, aucune section
rangee, pas de sortie en cours). La tranche se mesure depuis la mise en
route (`slice_start_ns`, posee a la commutation et au reveil d'idle).
`[SCHED-NG-REVEIL]` publie `preempt_noyau_quantum` (`log_reveil` n'etait
appele nulle part). Garde `verifie-quantum-noyau.py` (5 negatifs).

| QEMU, scheduler-ng-banc, 15 demarrages SMP1/4/8 par image | sans | avec |
|---|---|---|
| preemptions au quantum par demarrage | — | 1-2 (SMP1), 1-5 (SMP4), 4-10 (SMP8) |
| plus longue attente derriere un fil noyau | 427 ms (`usb-hid`, pendant le gel ci-dessous) | 64 ms (`services-metrics`) |
| `fork-exit-wait4` p99, mediane SMP1 / SMP4 / SMP8 (ms) | 23,3 / 30,1 / 24,8 | 24,0 / 26,4 / 30,0 |
| echecs / perdus / ressuscites | 0 / 0 / 0 | 0 / 0 / 0 |

L'episode d'origine (732 ms) est rare -- un demarrage sur une vingtaine --
et 15 demarrages ne suffisent pas a le declarer disparu : la preuve est
celle du mecanisme (il se declenche, sans regression), pas d'une frequence.

### Ouvert : gel de toute la machine pendant ~2 s

Dans 2 demarrages SMP4/8 sur 10, avant comme apres ce lot, TOUS les coeurs
s'arretent ensemble 2 a 4,5 s : `PERF_FORK ... reste_us=2025745`,
`HID_LATENCY_SPIKE delta_us=2024460`, attentes vives simultanees sur les
quatre coeurs occupes a des choses differentes (write, fork, exit_group,
fil noyau). Ni un releve IRQ (56 ms juste avant), ni une attente de verrou
tournant (`[SMP-SPIN]` muet), ni un shootdown TLB (`tlb_relances=0`). A
attribuer par une sonde dediee.

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
