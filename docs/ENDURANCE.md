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
| SMP1, SMP2 | voir le run | | |

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

### 3.2 Attentes pretes de ~2 s derriere le disque (OUVERT)

Residu SMP4/SMP8 apres 3.1 : la pire attente (usb-hid interactive, enfants
de disque-probe) est sur le coeur qui execute une E/S disque. Mecanisme
mesure (voir `MESURE_DEMARRAGE_A_FROID.md` section 17) :

* le controleur ATA est protege par un `SpinLock` ; un `SpinLock` rend le
  coeur NON PREEMPTIBLE des l'attente ;
* `ata::read`/`write` gardent ce verrou sur TOUS les lots d'une requete, et
  `write` y ajoute `FLUSH CACHE` ; `fsync` sur `/persist` reecrit un
  instantane sous ce verrou ;
* `ata_max_ns` observe : 452 ms a 1,6 s d'attente unique ; `ata_wait_ns`
  cumule 41-46 s sur un cycle.

Piste (non realisee, audit requis) : verrou dormant pour le controleur, ou
relachement entre lots. A trancher apres l'attribution Ladybird.

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
