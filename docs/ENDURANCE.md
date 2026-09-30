# Endurance : ce que le banc execute, ce qu'il mesure, ce qu'il a trouve

Etat au 2026-09-30, branche `claude/ladybird-observability-performance`.

## 1. Le banc

`.github/workflows/endurance.yml` -- hebdomadaire et a la demande
(`minutes_par_tranche`), quatre tranches paralleles SMP1/2/4/8.

Chaque tranche :

1. construit le disque de scenario **depuis les sources du commit**
   (`tools/ci/fabrique-scenario-endurance.sh`, sondes musl hors arbre) ;
2. enchaine des cycles QEMU (`tools/ci/reliability/soak.py --disque
   --require-fichier`) ; chaque cycle demarre le noyau, joue l'autorun,
   s'eteint ;
3. rend son verdict avec `tools/ci/reliability/budgets_endurance.py`.

### Ce que chaque cycle execute

| famille | charge reelle (anneau 3) | marqueur exige |
|---|---|---|
| releve | `smpstat` au depart et a l'arrivee | `[BKL-DOMAINES]`, `[SCHED-NG-LAT]`, `[SCHED-NG-CENTILES]`, `[SCHED-NG-FILE]` |
| memoire / SMP | `mmstress 4 512 4`, `unrelated`, `aba`, `churn` | `ENDURANCE_MEMOIRE_OK` |
| ordonnanceur | `ordonnanceur-probe 8` (A/B interactif vs normal sous 8 calculs) | `ENDURANCE_ORDONNANCEUR_OK` |
| stockage | `disque-probe` sur 6 Mio adosses au disque, `wal-probe` | `ENDURANCE_STOCKAGE_OK` |
| processus | `session-probe 4`, `verrous-probe`, `poll-bkl-probe` | `ENDURANCE_PROCESSUS_OK` |
| fin | -- | `ENDURANCE_CYCLE_FIN`, `=== AUTORUN FIN === statut=0` |

Une famille qui se fige (interblocage, famine) ou echoue ne publie pas son
marqueur : le cycle est rouge meme sans panique. `logscan` detecte en plus
panique, faute fatale, violation BKL/lockdep, UAF.

### Le verdict, en quatre reponses separees

1. **CAMPAGNE** -- cycles executes / reussis, tronques evites ;
2. **BUDGETS** -- evalues trace par trace par `check_budgets.py --journal`,
   avec `N/M grandeurs mesurees` ;
3. **METRIQUES ABSENTES** -- requises (faute) et optionnelles (annoncees) ;
4. **NOYAU** -- fautes fatales, marqueurs absents, statut de l'autorun,
   `RESULTAT` des sondes, `[SCHED-NG-PIRE]`.

Fail-closed : resume absent, zero cycle, pas de disque de scenario, trace
absente ou vide, cycle en echec, budget depasse ou requis absent => rouge.

### Ce qui empeche le banc de redevenir muet

Le workflow cherchait `cycle-*/serial.log` quand `run_one` ecrivait
`smp{N}.log` : l'etape des budgets n'a jamais lu une trace. Le nom n'est plus
connu qu'a un endroit (`qemu_matrix.journal_du_cycle`), le consommateur suit
le champ `log` de `summary.json`, et `test_traces_endurance.py` (11 tests)
fait tourner le VRAI `soak.py` + `run_one` avec un faux emulateur puis le VRAI
consommateur : un renommage d'un seul cote rend le test rouge.
`check_budgets.py --journal` sur un fichier introuvable rend 2 (rendait 0).

### Ce que le banc NE couvre PAS

Ladybird ne tourne pas dans ce scenario : `ladybird_relances_refusees` est
annoncee absente. Les relances et la stabilite sous charge Ladybird relevent
de `ladybird-native-browser.yml`.

## 2. Premiere campagne CI (run 36716086323, commit 1909e792, tranches de 30 min)

| tranche | campagne | budgets | noyau |
|---|---|---|---|
| SMP8 | 1 cycle (arret au 1er rouge) | evalues 12/14 ; rouges : interactive p99 134 ms, interactive max 979 ms, `bkl_attente_max` 66 ms | ECHEC : ordonnanceur-probe A/B 2/4, statut 1 |
| SMP4 | 19 cycles | evalues sur les 19 ; rouges a chaque cycle (`ready_latency_max` ~2 s isole, interactive p99 134 ms) | cycles 1-18 OK, cycle 19 marqueur absent ; 0 faute fatale |
| SMP2 | 35 cycles | evalues sur les 35 ; rouges a chaque cycle | 35/35 OK, 0 faute fatale |
| SMP1 | tous les cycles de la tranche | **tenus** : verdict VERT | OK, 0 faute fatale |

Aucune panique, aucune faute fatale. Les rouges sont des budgets de latence
et une sonde A/B : de vrais defauts, non relaches.

## 3. Ce que le banc a trouve

### 3.1 Famine par numero d'emplacement (CORRIGE, e3288efd)

`[SCHED-NG-PIRE]` (instrumentation 3bf9a86e) a attribue 26,5 s d'attente
prete a l'enfant 41 de `disque-probe` (SMP1) : quatre enfants forkes a
24,3 s servis un par un a la sortie de leur aine. Le tourniquet de
`BandeFile` avancait par MOT de 64 emplacements et servait toujours le plus
petit bit : sous 64 taches, priorite stricte par numero d'emplacement.

Correction : curseur au bit pres, parcours circulaire. Preuve hote (module de
production) : 16/18 -> 18/18. QEMU SMP1 : `ready_latency_max` 26 523 ->
410 ms, ordonnanceur-probe echec -> 0/4. SMP4 : interactif sous charge p99
13,8 -> 3,1 ms.

### 3.2 Attentes pretes de ~2 s derriere le disque (CORRIGE en partie, 19f53545)

Residu SMP4/SMP8 apres 3.1 : la pire attente (usb-hid interactive, enfants
de disque-probe) etait sur le coeur qui executait une E/S disque.

Cause mesuree : le controleur ATA etait garde par un `SpinLock`, qui rend le
coeur NON PREEMPTIBLE des l'attente ; le transfert PIO dure (un lot de 256
secteurs sous le meme verrou, `FLUSH CACHE` a l'ecriture). Les coeurs en
attente tournaient a vide, taches pretes derriere eux, et sous TCG volaient
le processeur hote a l'emulation du detenteur. Au passage, `probe()` prenait
ce verrou a chaque E/S pour lire un drapeau (db60f400, double file).

Trois bras mesures, meme scenario :

| | spinlock | SleepMutex | verrou a tickets dormant (retenu) |
|---|---|---|---|
| cycles OK | 1/2 | 1/4 | 4/4 |
| `ready_latency_max` SMP4 | 2 040 / 2 306 ms | 194-237 ms | 447 / 314 ms |
| SMP8 / SMP1 | -- | 134 / 574 ms | 61 / 258 ms |
| p99 interactif | 134 / 268 ms | 33,5 ms | 33,5 ms |
| pire attente controleur | 0,76 / 1,15 s | 6,9-7,7 s | 0,26-0,33 s |
| debit controleur | ~1,5 Mio/s | ~2,8 Mio/s | ~3 Mio/s |

`SleepMutex` n'est pas equitable (un arrivant passe devant le reveille) : une
faute de page pouvait attendre 7 s. Le verrou a tickets sert dans l'ordre.

Puis des points de preemption dans le transfert PIO et l'attente de commande
(`BOUCHAUD_ATA_POINT_SUR_V1`) : le coeur qui transfere n'est plus monopolise.

| | FIFO seul | FIFO + points surs |
|---|---|---|
| `ready_latency_max` SMP4 | 447 / 314 ms | 56-90 ms (n=5) |
| SMP1 / SMP8 | 258 / 61 ms | 104 / 64 ms |
| interactive max | 314-447 ms | 47-90 ms |

Les budgets `ready_latency_*` sont TENUS sur ces cycles. Reste, NON relache :
`bkl_attente_max` 47-70 ms (budget 50, deja 36-66 avant ces commits) ; une
tenue BKL de 157 ms (fcntl, cpu0) sur 1 cycle sur 5, non reproduite.

### 3.3 Sonde A/B de l'ordonnanceur a SMP>=2

Apres 3.1, l'ecart A/B a SMP4 est de 144 us sur le maximum de 60 echantillons
(2 965 vs 3 109 us) ; avant, 5 a 6 fois (2,3 vs 13,8 ms). Le verdict de la
sonde n'a pas ete modifie ; il reste bloquant et rapporte tel quel.

## 4. Commandes

    tools/ci/fabrique-scenario-endurance.sh TRAVAIL
    PYTHONPATH=tools/ci/reliability python3 tools/ci/reliability/soak.py \
        target/x86_64-bouchaud_os/debug/bootimage-bouchaud-os.bin --cpus 4 \
        --duration-seconds 610 --cycle-seconds 600 --memory-mb 4096 \
        --out-dir SORTIE --disque TRAVAIL/endurance.img \
        --require-fichier TRAVAIL/marqueurs-endurance.txt
    python3 tools/ci/reliability/budgets_endurance.py SORTIE
    python3 tools/ci/attribue-lectures-disque.py SORTIE/cycle-0001/smp4.log

## 5. Deuxieme campagne CI (run 36723718532, commit 220bdf27 : FIFO + points surs)

| tranche | run #6 (1909e792, avant les correctifs ATA) | run #8 (220bdf27) |
|---|---|---|
| SMP1 | VERT | 32/32 cycles noyau OK ; budgets tenus 30/32 -- cycle 26 : `bkl_max_hold` 321 ms (exit_group), `ready_latency_max` 323 ms |
| SMP2 | 35/35 cycles noyau OK, budgets rouges 35/35 | cycles noyau OK, budgets rouges 1 cycle (27) |
| SMP4 | 19 cycles, budgets rouges 19/19, cycle 19 noyau KO | cycles noyau OK, budgets rouges 1 cycle (4) |
| SMP8 | 1 cycle : budgets rouges (p99 interactif 134 ms, max 979 ms) + sonde A/B | 1 cycle : budgets TENUS ; sonde A/B 2/4 (seul echec) |

Nouveau rouge a SMP1 (2 cycles sur 32, aucun avant) : hypothese a verifier, une
inversion de priorite -- un detenteur du controleur preempte a un point sur,
un detenteur du BKL qui a besoin du disque parque derriere lui. Ouvert.

### 5.1 Le rouge SMP1 : l'inversion est REFUTEE

Diagnostic `ATA_ATTENTE_LONGUE` (attentes du controleur >= 100 ms, avec la
profondeur du gros verrou chez l'attendant), 21 cycles SMP1 locaux :

* 21/21 cycles noyau OK ; budgets tenus 20/21 ;
* TOUTES les attentes longues ont `bkl_profondeur=0` : file FIFO ordinaire
  entre les cinq lecteurs de `disque-probe` (110-153 ms, 4-5 en file). Aucun
  detenteur du BKL ne parque derriere le controleur ;
* le cycle rouge : `ready_latency_max` 336 ms, un seul reveil tardif (p99
  16,8 ms), un lecteur en classe normale ; BKL tenu 37 ms au plus.

Hypothese restante, NON testee : le reveil collectif (`wake_all`) du verrou a
tickets reveille tous les attendants a chaque liberation ; a un seul coeur,
ce sont des commutations en plus. Un reveil cible du ticket suivant demande
une primitive que `WaitQueue` n'offre pas sans risque de reveil perdu.
Ouvert ; budget non relache.
