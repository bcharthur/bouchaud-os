# Scheduler NG — ligne de base (phase 0)

BOUCHAUD_SCHEDULER_NG_BANC_V1 — 1er octobre 2026, noyau `9fcc13c5`
(`stable/continuation-green-20261001`), avant toute modification de
l'ordonnanceur.

Chaque phase du chantier Scheduler NG se juge contre ces chiffres ; les seuils
de la phase 13 en derivent.

## 1. Le banc

`tools/userland/scheduler-ng-banc.c`, binaire musl ring 3, une ligne
`SNG v=1 sec=...` (cle=valeur) par mesure, agregees par
`tools/ci/scheduler_ng_baseline.py` (mediane et etendue par configuration).

| section | ce qu'elle mesure |
|---|---|
| A — latence interactive | retard de reveil d'une tache periodique (10 ms, 200 reveils) au repos, puis sous 1xC et 2xC calculs (C = coeurs, plafonne a 16), classe normale puis interactive (`nice -5`) |
| B — equite CPU | 2, 4, 8, 16 calculs identiques pendant 2 s : indice de Jain, rapport min/max des tours, plus long « trou » (temps ou un calcul PRET n'a pas tourne) |
| C — charge mixte | tache interactive + 2xC calculs + ping-pong par tube (300 allers-retours) + fork/exit en continu |
| D — cycle de vie | fork/exit/wait4 (100), fork/exec/exit (20), mort etrangere (`exit_group` d'un fil pendant que trois freres dorment dans nanosleep / futex / read — 30 de chaque), kill d'un endormi (20), fin de fichier avant recolte (10), racine morte avant ses descendants (5), ping-pong futex (500) |

Invariants exiges (pas des seuils) : tout fils recolte avant son echeance
(`perdus`), aucun fil d'un processus mort ne s'execute apres sa recolte
(`ressuscites` : compteurs en memoire partagee relus 30 ms apres la recolte),
aucun reveil futex perdu, tout calcul progresse. Une echeance depassee ne fige
jamais le banc : elle est comptee.

Le noyau publie en plus, par `smpstat` avant et apres le banc, ses compteurs
cumules : latence pret-a-tourner par classe (`[SCHED-NG-LAT]`,
`[SCHED-NG-CENTILES]`), preemptions (`[SCHED-NG-PREEMPT]`), files
(`[SCHED-NG-FILE]`), vols et migrations (`[SMP-LOAD]`).

## 2. Protocole et classe de preuve

QEMU TCG (preuve **QEMU**, pas physique), `soak.py` : 3 demarrages par
configuration SMP1, 2, 4, 8, 16, 2 Gio, disque de scenario
`smpstat ; /bin/scheduler-ng-banc ; smpstat`. Hote : 4 coeurs. **SMP8 et SMP16
sont sur-souscrits** (8 et 16 vCPU sur 4 coeurs hote) : leurs latences
mesurent aussi l'hote ; elles servent a la correction et aux tendances, pas a
des seuils absolus. Identite verifiee : `BOUCHAUD_BUILD commit=9fcc13c59d70`.

Le banc est lance par `run` : sa tache racine est epinglee au coeur du
lancement (coeur zero). Les fils forkes ont l'affinite machine.

## 3. Resultats (mediane [min-max] de 3 demarrages)

| grandeur | SMP1 | SMP2 | SMP4 | SMP8 | SMP16 |
|---|---|---|---|---|---|
| A repos p99 (us) | 2006 [1533-6658] | 3057 [1984-4219] | 3548 [2034-6926] | 7300 [4335-25971] | 2365 [1625-2449] |
| A 1xC normale p99 (us) | 5676 | 6038 | 5975 | 19221 | 101655 |
| A 1xC interactive p99 (us) | 4291 | 3734 | 5882 | 18832 | 62339 |
| A 2xC normale p99 (us) | 2477 | 2400 | 3522 | 23263 | 101655 |
| A 2xC interactive p99 (us) | 2170 | 4525 | 5192 | 15485 | 62339 |
| A 2xC interactive max (us) | 2677 | 6065 | 28448 [2007-64585] | 15618 | 595073 |
| B 16 calculs, Jain (‰) | 971 | 998 | 930 [922-947] | 965 | 993 |
| B 16 calculs, min/max (‰) | 564 | 903 | 482 [436-485] | 494 | 744 |
| B 16 calculs, trou max (us) | 18663 | 65936 | 109227 | 232265 | 74580 |
| C reveil p99 (us) | 14191 | 14979 | 41917 | 30915 | 78022 |
| C ping-pong tube p99 (us) | 11596 | 20197 | 19799 | 38099 | 31763 |
| C fork/exit en 4 s | 307 | 192 | 182 | 114 | 64 |
| D fork/exit/wait4 p99 (us) | 32535 | 26379 | 47843 | 49301 | 55486 |
| D fork/exec/exit p99 (us) | 34390 | 48539 | 45442 | 51259 | 44583 |
| D mort etrangere nanosleep p99 (us) | 9371 | 16584 | 35089 | 17453 | 20677 |
| D kill d'un endormi p99 (us) | 8534 | 25821 | 19438 | 30990 | 64486 |
| D ping-pong futex, echange moyen (us) | 435 | 957 | 1123 | 2423 | 4322 |
| **perdus** | 5 | 5 | 5 | 5 | 5 |
| **ressuscites** | 0 | 222 [210-256] | 264 [258-268] | 269 [268-270] | 270 [268-270] |
| reveils futex perdus | 0 | 0 | 0 | 0 | 0 |
| noyau : attente pret->elu p99, normale (us) | 4194 | 2097 | 2097 | 8389 | 16777 |
| noyau : attente pret->elu max (ms) | 526 | 80 | 788 [120-1420] | 424 | 656 [655-13892] |
| noyau : attente de service de preemption max (ms) | 526 | 2172 | 2589 | 3992 | 7796 |
| noyau : vols reussis / tentes | 0/0 | 4849/7974 | 12949/23202 | 24253/42588 | 36714/88955 |

Les centiles noyau sont des bornes de seau puissance de deux (p99 = 4194304 ns
signifie « dans le seau 2-4 ms »). Tableau complet, compteurs noyau compris :
sortie de `scheduler_ng_baseline.py` sur les 15 journaux.

## 4. Ce que la ligne de base revele

Classes : **[noyau]** defaut du noyau a corriger dans le chantier ;
**[ABI]** conformite ; **[mesure]** a attribuer avant de corriger.

1. **[noyau] Resurrection de fils tues (`ressuscites` 210-270 des SMP2).**
   Un `exit_group` marque les freres zombie ; celui qui entrait dans
   `nanosleep`, un futex ou un `read` ecrasait `Zombie` par `Blocked` (simple
   store), etait remis `Ready` par son echeance ou son reveil, et retournait en
   espace utilisateur apres la recolte de son processus. En fin de banc,
   `[SMP-LOAD]` montre encore ces fils sur les coeurs. Nul a SMP1 : le frere
   ne peut pas s'executer pendant la sortie. Phase 1.
2. **[noyau] Fin de fichier jamais livree apres la mort d'un ecrivain.**
   `eof-avant-recolte` 0/10 et `racine-avant-descendants` 0/5 a TOUS les SMP :
   les descripteurs d'un processus mort ne sont pas fermes a sa mort (ni a sa
   recolte : la lecture apres `wait4` ne voit pas non plus la fin). Un
   processus orphelin garde ses tubes ouverts pour toujours. Ce sont les 5
   `perdus`. Phase 9 (semantique de sortie).
3. **[ABI] `sched_getaffinity` rend le masque 1.** `coeurs_sysconf=1` a tous
   les SMP alors que `/sys/devices/system/cpu/online` dit N : musl calcule
   `sysconf(_SC_NPROCESSORS_ONLN)` par `sched_getaffinity`. Tout pool de fils
   d'un programme musl — dont ceux du navigateur — est dimensionne a UN fil.
   Phase 4 (topologie generique).
4. **[ABI] `MAP_SHARED|MAP_ANONYMOUS` n'est pas partage a travers `fork`**
   (`partage_fork=2` : le banc s'est replie sur `memfd`). Hors ordonnanceur ;
   consigne.
5. **[mesure] Attentes de plusieurs centaines de millisecondes.** Attente
   pret->elu max 0,1 a 1,4 s des SMP1 ; a SMP4 #2, le reveil interactif ET
   un calcul ont attendu 1,42 s au meme moment (`reveil_max_us=1421512`,
   `calcul_trou_max_us=1421621`). L'attente de service d'une preemption
   demandee atteint 2 a 8 s. Une partie vient des fils ressuscites (fin de
   banc) ; le reste est a attribuer. Phase 3.
6. **[mesure] Equite a 16 calculs.** Rapport min/max 0,48-0,56 a SMP1/4/8
   (un calcul fait deux fois moins que le meilleur), trou max 19 ms (SMP1) a
   232 ms (SMP8). Phase 2 (file NG) et 5 (equilibrage).
7. **[mesure] La classe interactive ne protege pas sous charge.** A 1xC et
   2xC, p99 interactif ≈ p99 normal ; a SMP16 le max interactif atteint 595 ms.
   Phases 3 et 7.

## 5. Seuils

Aucun seuil n'est pose a la phase 0. Les invariants (`perdus`, `ressuscites`,
reveils perdus, calcul affame) sont absolus et doivent tomber a zero par
correction, pas par tolerance. Les seuils de latence et d'equite seront
derives de ce tableau a la phase 13, configuration par configuration, sur les
configurations non sur-souscrites (SMP1/2/4).
