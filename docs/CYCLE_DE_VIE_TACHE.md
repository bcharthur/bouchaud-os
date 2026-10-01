# Cycle de vie d'une tache : un zombie ne ressuscite jamais

BOUCHAUD_CYCLE_DE_VIE_V1 — Scheduler NG, phase 1 — 1er octobre 2026,
base `b8176797`.

## 1. Le defaut

Ligne de base (`docs/SCHEDULER_NG_BASELINE.md`, section D « mort etrangere ») :
un processus de quatre fils, dont trois dorment dans `nanosleep`, un futex ou
un `read`, et dont le quatrieme appelle `exit_group`. Trente compteurs par
attente, relus 30 ms apres la recolte du processus :

| SMP | 1 | 2 | 4 | 8 | 16 |
|---|---|---|---|---|---|
| compteurs de fils morts qui bougent encore | 0 | 222 | 264 | 269 | 270 |

En fin de banc, `[SMP-LOAD]` montre ces fils sur les coeurs, rattaches a des
processus deja recoltes. C'est aussi la cause du blocage de `session-probe`
observe pendant le lot de la continuation synchrone (§7 de
`docs/CONTINUATION_SYNCHRONE.md`).

Mecanisme, lu dans le code :

1. `exit_group` appelait `marque_zombie` sur chaque frere : un **simple store**
   `Zombie`, que le frere s'execute ou non ;
2. le frere, s'il etait en train d'entrer dans une attente, publiait ensuite
   son parking par un autre **simple store** : `Blocked` ecrasait `Zombie` ;
3. son echeance ou son reveil le remettait `Ready` (`Blocked -> Ready` par
   CAS : rien n'interdisait de partir d'un zombie ressuscite) ;
4. a la mise en route, le drapeau de retraite du coeur valait
   `etat == Zombie`, donc faux : la sortie d'appel systeme le laissait
   retourner en espace utilisateur.

Une seconde famille sortait du meme store : un frere tue pendant une attente
du NOYAU (verrou dormant, lecture disque) n'etait jamais reveille s'il etait
deja parque — en emportant le verrou — et ressuscitait s'il ne l'etait pas
encore.

## 2. La machine d'etat

```
           endort (la tache, CAS)
    Pret ───────────────────────────▶ Bloque
      ▲  ◀───────────────────────────   │
      │   reveille (reveilleur ou        │ tue_parquee (un tueur, CAS,
      │   annulation, CAS)               │  attente interruptible,
      │ meurt (la tache elle-meme)       │  tache hors de tout coeur)
      ▼                                  ▼
    Zombie ◀─────────────────────────────┘     absorbant
```

* `EtatAtomique` n'expose plus que ces quatre transitions (`modeles.rs`).
  `range` et `echange` ont disparu : il n'existe plus d'ecriture libre.
* Une tache **qui s'execute** n'est jamais passee a `Zombie` par une autre.
  Elle est **condamnee** (`Task::condamnee`) et meurt elle-meme a sa
  prochaine **frontiere** :
  - sortie d'appel systeme et de faute (`retire_current_if_zombie`) ;
  - preemption depuis l'espace utilisateur (`preempt_from_irq`) — le tueur
    envoie l'IPI qui l'y amene ;
  - premier passage en espace utilisateur (`task_trampoline`), pour un fil
    condamne avant d'avoir tourne ;
  - mise en route sur un coeur (drapeau de retraite pose si condamnee) ;
  - attente **interruptible** : elle y meurt au lieu d'y dormir.
* Le tueur (`condamne`, `comptabilite.rs`) applique `cycle_vie::action_tueur` :

  | ce qu'il voit | action |
  |---|---|
  | Zombie | rien |
  | Pret (sur un coeur ou en file) | frontiere ; IPI si elle tourne ailleurs |
  | Bloque, encore sur son coeur | `reveille` + publication : elle reboucle et voit sa condamnation |
  | Bloque, parquee, attente interruptible | `tue_parquee` (CAS) |
  | Bloque, parquee, attente noyau | rien : son evenement la ramenera a sa frontiere |

* **La fenetre du parking.** Le tueur publie `condamnee` puis lit l'etat ; la
  tache publie `Bloque` puis relit `condamnee`. Ordre total des deux cotes
  (`SeqCst`) : au moins l'un voit l'autre.
* **Interruptible ou noyau.** `WaitQueue::wait` / `wait_until` (descripteurs,
  futex, IPC native), `sleep_ticks` et `wait4` sont interruptibles. Les neuf
  attentes qui tiennent une ressource du noyau prennent `wait_noyau` : verrou
  dormant, page en chargement (cache de pages, memoire partagee x5), faute en
  cours, controleur ATA. Une tache condamnee les termine, puis meurt a sa
  frontiere.
* **`execve` et `exit_group`** attendent la mort de tous les freres condamnes
  (`attend_extinction_freres`, sommeil NON interruptible) : `execve` ne
  remplace l'image qu'apres — un frere qui termine une lecture disque ne
  reprend plus sur le nouvel espace —, et `exit_group` ne rapporte la fin du
  processus qu'apres — `wait4` ne rend plus un processus dont un fil
  s'execute encore, comme sous Linux. Au dela d'une seconde, la ligne
  `EXTINCTION_FRERES_LENTE` nomme le fil attendu ; l'attente continue. Une
  tache elle-meme condamnee pendant cette attente cesse d'attendre.

Aucun verrou : des CAS sur l'etat, un drapeau par tache, la porte de
transition par coeur comme avant.

## 3. Preuves

**Hote** — `tools/smp/test_cycle_vie.rs` explore TOUS les entrelacements de la
tache, du tueur et du reveilleur, dans quatre scenarios (nanosleep, futex sans
reveilleur, verrou dormant, calcul pur), sous l'ancien protocole et le nouveau,
avec les fonctions memes du noyau (`cycle_vie.rs`) :

| | etats | Zombie quitte | resurrection | mort perdue | double file |
|---|---|---|---|---|---|
| ancien, nanosleep | 40 | 2 | 1 | 1 | 0 |
| ancien, futex | 11 | 1 | 0 | 1 | 0 |
| ancien, verrou | 39 | 2 | 1 | 1 | 0 |
| nouveau, 4 scenarios | 53 / 21 / 47 / 11 | 0 | 0 | 0 | 0 |

**QEMU** — `scheduler-ng-banc`, 3 demarrages par configuration : voir §4.

**Gardes** — `tools/verifie-cycle-vie.py` (8 negatifs) ;
`tools/verifie-ordonnanceur-sans-bkl.py` suit le nom de la transition.

## 4. Mesures avant / apres

QEMU TCG, memes conditions que la ligne de base. Compteur `[SCHED-NG-CYCLE]` :
condamnations, tuees parquees, reveillees pour mourir, mortes a la frontiere,
mortes au parking, endormissements refuses (doit rester nul).

`scheduler-ng-banc`, 3 demarrages par configuration (12 par version) :

| | SMP1 | SMP2 | SMP4 | SMP8 |
|---|---|---|---|---|
| **ressuscites** — ligne de base | 0 | 222 [210-256] | 264 [258-268] | 269 [268-270] |
| ressuscites — phase 1, `exit_group` sans attente | 0 | 0 | 0, 0, 1 | 0, 1, 0 |
| **ressuscites — phase 1 (ce lot)** | 0 | 0 | 0 | 0 |
| mort etrangere nanosleep p99 (us), base -> phase 1 | 9371 -> 9691 | 16584 -> 18223 | 35089 -> 17026 | 17453 -> 14521 |
| kill d'un endormi p99 (us), base -> phase 1 | 8534 -> 8296 | 25821 -> 14235 | 19438 -> 16926 | 30990 -> 12869 |

Les deux compteurs de la version sans attente (un compteur, un demarrage sur
trois a SMP4 et SMP8) sont la fenetre que ferme l'attente des freres dans
`exit_group` : un frere en espace utilisateur sur un autre coeur executait
encore une instruction avant que l'IPI ne l'atteigne. Sur la version finale,
douze demarrages, zero.

Repartition des 271 morts imposees par demarrage (`[SCHED-NG-CYCLE]`) :

| SMP | tuees parquees | reveillees pour mourir | mortes a la frontiere | mortes au parking | endormissements refuses |
|---|---|---|---|---|---|
| 1 | 239-252 | 0 | 3-12 | 14-26 | 0 |
| 2 | 204-222 | 31-35 | 37-50 | 11-19 | 0 |
| 4 | 126-131 | 83-95 | 102-107 | 37 | 0 |
| 8 | 57-75 | 117-136 | 139-153 | 56-60 | 0 |

Plus le nombre de coeurs croit, plus les freres tournent encore au moment de
leur condamnation : la part des morts « a la frontiere » et « reveillees »
monte, celle des tuees parquees baisse. `endormissements_refuses` reste nul.

Autres suites sur ce noyau : `continuation-banc` SMP1/2/4/8 OK (0
`fil_mort=0`), `session-probe` revient a l'invite SMP1/4, `OS_PRIMITIVES_OK`,
`SYSTEM_HEALTH_OK`.

Un demarrage SMP4 de la version finale s'est fige sur une tache prete perdue
(`[SCHED-ORPHELINE]`) avant toute condamnation : course passation/reveil
pre-existante (deux rapports `[SCHED-ORPHELINE]` dans la ligne de base),
traitee par le lot suivant (BOUCHAUD_PASSATION_REVEIL_ORDONNES_V1).

## 5. Ce qui reste ouvert

* Une tache tuee **sur place** dans une attente interruptible ne deroule pas
  sa pile : les references qu'elle tient (un `Arc` de descripteur) fuient.
  C'etait deja le cas de toute tache passee `Zombie` pendant qu'elle dormait ;
  le corriger demande de rendre `EINTR` jusqu'a la sortie d'appel systeme
  (phase 9).
* Une fin de session (`tue_processus`, descendance de la racine) condamne
  sans attendre : un fil qui termine une attente noyau peut encore executer
  du code noyau apres le menage (jamais d'espace utilisateur).
* Sur le coeur zero, la preemption directe d'une tache utilisateur est
  differee (`BSP_DEFER_DIRECT_IRQ_PREEMPT_V8`) : un frere condamne qui calcule
  sans appel systeme sur le BSP ne meurt qu'a son prochain appel. Meme
  dependance qu'avant ce lot.
* Les fins de fichier non livrees apres la mort d'un ecrivain (5 `perdus`
  dans la ligne de base) relevent de la fermeture des descripteurs : phase 9.

## 6. Passation et reveil (BOUCHAUD_PASSATION_REVEIL_ORDONNES_V1)

Le demarrage SMP4 fige du §4 : une tache `Ready`, hors de tout coeur, dans
aucune file. Une tache bloquee reveillee PENDANT sa commutation de sortie est
publiee soit par la passation (`complete_switch_handoff` : ecrit `on_cpu = -1`,
`switching_out = false`, puis lit l'etat), soit par le reveilleur
(`publish_ready` : apres le CAS `Blocked -> Ready`, lit `on_cpu` /
`switching_out`). Motif « store buffer » : en x86-TSO, la lecture de chaque
cote peut passer devant sa propre ecriture ; la passation lit `Blocked`, le
reveilleur lit `switching_out == true`, et personne ne publie. Une barriere
`SeqCst` de chaque cote, entre l'ecriture et la lecture, l'interdit (au pire
la tache est publiee deux fois ; la file deduplique).

* hote `tools/smp/test_passation_reveil.rs` : modele x86-TSO exhaustif
  (tampons d'ecriture, vidage non deterministe, instructions verrouillees) —
  l'ancien protocole a une execution qui perd la tache, le nouveau aucune ;
  sur de vrais fils de l'hote, 31 pertes sur 200 000 pour l'ancien, 0 pour
  le nouveau ;
* QEMU `scheduler-ng-banc`, 4 x SMP1/2/4/8 : 16/16 complets, zero rapport
  `[SCHED-ORPHELINE]` (ligne de base : 2 rapports sur 15 demarrages ;
  phase 1 seule : 1 demarrage fige sur 12) ;
* garde `tools/verifie-passation-reveil.py` (3 negatifs).

## 7. Descripteurs fermes a la mort (BOUCHAUD_DESCRIPTEURS_A_LA_MORT_V1)

Ligne de base, tous les SMP : `eof-avant-recolte` 0/10, `racine-avant-descendants`
0/5. La table de descripteurs vit dans le `Process`, que l'emplacement de la
tache zombie tient jusqu'a son recyclage : un tube dont l'ecrivain etait mort
ne rendait jamais la fin de fichier, ni avant ni apres `wait4`, et les tubes
d'un processus orphelin restaient ouverts pour toujours.

Les descripteurs se ferment desormais a la mort du dernier fil (`exit_current`)
et a une mort imposee (`tue_processus`), AVANT de prevenir le parent — comme
`exit_files` precede `exit_notify` sous Linux. La table est videe sous son
verrou, les descripteurs fermes hors du verrou (fermer un tube reveille ses
lecteurs). Elle n'est jamais partagee entre processus (`fork` la copie).

QEMU `scheduler-ng-banc`, 3 x SMP1/2/4/8/16 : `eof-avant-recolte` 10/10 et
`racine-avant-descendants` 5/5 sur les 15 demarrages, `perdus` 0, `echecs` 0
— le banc passe entierement pour la premiere fois, et sa duree tombe de
72-90 s a 37-50 s (plus d'echeance de 2-3 s sur les fins de fichier
perdues). Garde `tools/verifie-descripteurs-a-la-mort.py` (3 negatifs).
