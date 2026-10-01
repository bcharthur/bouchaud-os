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
| quantum des fils noyau | `1df8a88b`, RETIRE par `000a99ef` | fil noyau coupe n'importe ou : contrats de non-preemption violes | panic `runtime > fenetre` (lecteur du registre commute) ; voir §3 |
| preemption noyau sure | `000a99ef` | predicat aveugle au garde de lecture du registre et au shootdown TLB en vol | test hote exhaustif 256 cas ; garde 8 negatifs |
| sonde de gel (obs.) | `25d978e2` | gel global sans coupable | gel = attente des lecteurs du registre, a la ms |
| releves hors hard IRQ + ligne serie IRQ masquees | (ce lot) | cycle registre / jeton serie / tic, rompu seulement par la borne du jeton | SMP8 : gels >= 200 ms 4/9 (pire 24,2 s) -> 0/15 |

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

**RETIRE** (`000a99ef`, BOUCHAUD_PREEMPTION_NOYAU_SURE_V1). Couper un fil
noyau a n'importe quel point IF=1 viole les contrats de non-preemption du
noyau (garde de lecture du registre, emplacement TLB par coeur, compositeur).
Mesure : panic `task: runtime > fenetre tid=104` dans `services-metrics`
commute sous garde de lecture. La decision restante (demande ciblee seule)
exige en plus : aucune lecture du registre, aucun shootdown en vol, IF
masque (`kernel::preemption_noyau`, test hote exhaustif). Le texte ci-dessous
decrit le lot retire.

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

### Le gel global (BOUCHAUD_SONDE_GEL_V1, BOUCHAUD_RELEVES_HORS_IRQ_V1, BOUCHAUD_JETON_SERIE_CONTEXTE_V1)

Sonde par coeur (`sonde_gel.rs`) : trou entre deux tics d'un meme coeur,
RIP/tache/site avant et apres, tics PIT livres ; sonde hote
(`tools/ci/sonde-gel`) : fils de QEMU, schedstat, futex.

Ce qu'elle a etabli (QEMU SMP8) :

1. pendant un gel, 7 ou 8 coeurs sans tic au meme instant, la plupart
   immobiles DANS leur gestionnaire de tic ; cote hote les 8 vCPU tournent
   (R, CPU consomme, aucun futex commun) : ni pause QEMU, ni famine hote,
   ni verrou global de l'emulateur ;
2. duree du gel = attente des lecteurs du registre par l'ecrivain (`fork`),
   a la milliseconde (227/233, 257/260, 24210/24212 ms) ;
3. aucun garde de lecture ne traverse une commutation, aucun compte ne
   deborde ; la destruction de l'incarnation recyclee coute 3-9 ms : ni
   l'un ni l'autre n'est la cause ;
4. tout gel >= 250 ms coincide avec une attente du jeton serie allee a sa
   borne (100 000 tours, ~250 ms sous TCG) : 54 bornes -> 24,2 s.

Le cycle : une tache tient un garde de lecture et imprime (attend le
jeton) ; le detenteur du jeton, interruptions ouvertes, prend un tic dont le
gestionnaire demande une NOUVELLE lecture -- refusee, un ecrivain attend ;
l'ecrivain attend la premiere tache. Seule la borne du jeton le rompt.

Correctifs : le hard IRQ ne forme plus de releve (capture, `diag-noyau`
imprime) et une ligne serie s'emet interruptions masquees (son detenteur ne
peut plus etre interrompu ni preempte).

| QEMU SMP8, scheduler-ng-banc | instrumente seul | A (releves hors IRQ) | A+B (+ ligne IRQ masquees) |
|---|---|---|---|
| demarrages | 9 | 10 | 15 |
| gels >= 200 ms | 4 (pire 24 212 ms) | 0 | 0 |
| pire attente des lecteurs | 24 210 ms | 18 ms | 12 ms |
| bornes du jeton serie | 56 | 0 | 1 (sans gel) |
| echecs / perdus / ressuscites / panics | 0 | 0 (1 demarrage sans ligne FIN, a analyser) | 0 |

A seul suffit sur 10 demarrages : il retire du hard IRQ les ecrivains
serie (veille, releve) qui fermaient le cycle. B ferme le cycle par
construction (detenteur du jeton non interruptible) et supprime la
corruption des lignes (texte d'IRQ insere au milieu d'une ligne, mesure
avant). Classe de preuve : QEMU.

Ouvert : la borne du jeton atteinte une fois sous A+B sans gel ; le
demarrage A sans `FIN` ; usb-hid 732 ms (quantum retire) ; preemption ciblee
du compositeur (cas preexistant).

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
