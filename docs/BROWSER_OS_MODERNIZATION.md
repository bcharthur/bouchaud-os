# Ce que le noyau sait deja faire — et ce qui manque au renderer separe

*Etat au 11 aout 2026, revise apres les correctifs qu'il avait lui-meme
demandes. Aucun processus de rendu n'existe encore : ce document mesure le
terrain avant d'y construire. Les sections 1 a 9 decrivent l'etat courant ;
le verdict de fin dit ce qu'un renderer y trouverait.*

## Pourquoi ce document

Le navigateur est aujourd'hui **un seul processus** : `/bo-navigateur`, un ELF
statique qui contient Qt, CPython, QuickJS et ffmpeg. Une page qui fait boucler
le moteur fige l'interface ; une page qui epuise la memoire tue l'application
entiere. C'est exactement le probleme que l'architecture multi-processus de
Chromium et de WebKit resout — et la question posee ici est de savoir si ce
noyau peut la porter.

La reponse courte est **oui sur les trois axes** : un renderer separe
n'emporterait plus le navigateur en plantant, ni la machine en fuyant
(`RLIMIT_AS`), ni l'interface en calculant (deux classes d'ordonnancement). Le
dernier point etait le manque que la premiere version de ce document
identifiait comme decisif ; il est leve. Le reste du document dit comment, avec
les lignes de code a l'appui.

La methode est celle des autres audits du depot : lire le dispatch d'appels
systeme, lire les implementations, et ne rien affirmer qui ne soit verifiable.
Quand une sonde existante prouve deja la chose a l'execution, elle est citee —
`tools/userland/shm-probe.c` couvre a lui seul la chaine memoire partagee, et
`tools/test.sh:92` la joue a chaque passage.

---

## 1. Processus

| Appel | Etat | Ou |
|---|---|---|
| `fork` / `vfork` | complet, copie **immediate** | `abi/proc.rs:25` |
| `execve` | complet, y compris `ld.so` | `abi/proc.rs:112` |
| `wait4` | complet, bloquant et `WNOHANG` | `abi/proc.rs:216` |
| `clone` sans `CLONE_THREAD` | **`ENOSYS`** | `abi/mod.rs:746` |
| `waitid`, `pidfd_open` | absents | — |
| groupes de processus | factices | `abi/mod.rs:406`, `abi/proc.rs:363` |

Le cycle `fork` → `execve` → `wait4` → `SIGCHLD` est entier et coherent : la
table de descripteurs est clonee (donc partagee), les signaux en attente ne sont
pas herites, l'enfant reprend avec `rax = 0` et sa base FS suivie — sans quoi la
libc dereferencerait `%fs:0` a l'adresse nulle (`abi/proc.rs:81`). Lancer un
processus de rendu depuis le navigateur fonctionne aujourd'hui.

**Deux reserves, et elles ne sont pas theoriques.**

La premiere : `fork` copie l'espace d'adressage **entierement et tout de suite**
(`AddressSpace::duplicate`). Il n'y a pas de copie a l'ecriture. Forker un
processus qui porte Qt, CPython et une page chargee double sa memoire residente
a l'instant precis ou la memoire est la plus rare. La parade est connue et
n'exige rien du noyau : c'est le *zygote* de Chromium — forker **tot**, avant
que le tas ne grossisse, et garder le processus fils en attente d'un ordre.
L'architecture doit donc etre concue autour de cette contrainte plutot que la
decouvrir en production.

La seconde : `clone` refuse tout ce qui n'est pas `CLONE_VM|CLONE_THREAD`. Or
`posix_spawn` de musl passe par `clone(CLONE_VM|CLONE_VFORK|SIGCHLD)` — donc
`posix_spawn` echoue en `ENOSYS`. Le chemin `fork` + `execve` reste disponible
et c'est celui qu'emploie `subprocess` de CPython par defaut (`close_fds=True`),
mais toute bibliotheque qui tend la main vers `posix_spawn` se heurte a un mur.

Enfin, `kill(pid <= 0)` s'envoie le signal **a soi-meme** (`abi/proc.rs:363`) et
`setpgid`/`setsid` rendent 0 sans rien faire. « Tuer tout le sous-arbre d'un
renderer » n'est pas exprimable ; il faut suivre les pid soi-meme. Pour un
modele plat — un renderer par onglet, pas de petits-enfants — c'est sans
consequence.

## 2. Threads

Complet, et deja eprouve : `clone(CLONE_VM|CLONE_THREAD|CLONE_SETTLS)` avec
`CLONE_PARENT_SETTID`, `CLONE_CHILD_SETTID` et `CLONE_CHILD_CLEARTID`
(`abi/mod.rs:732`). La pile de l'enfant est reprise **telle quelle**, sans
realignement, parce que le trampoline `__clone` de musl a deja empile l'argument
du thread juste sous le pointeur transmis — un arrondi appellerait une adresse
arbitraire.

Ce chemin porte deja `pthread_create` de Qt, celui de CPython, et le pool de
threads reseau du moteur (`moteur/js.py`, `_op_requete`). Il n'est pas a
demontrer : il tourne.

`execve` termine les threads freres avant de remplacer l'image
(`abi/proc.rs:147`), ce qui est la semantique POSIX et evite qu'un thread
survivant s'execute dans un espace d'adressage detruit.

## 3. IPC

| Mecanisme | Etat | Ou |
|---|---|---|
| `pipe` / `pipe2` | complet, `O_CLOEXEC` compris | `abi/file.rs` |
| `socketpair` | complet, **bidirectionnel** | `abi/net.rs:780` |
| `sendmsg` / `recvmsg` | complet | `abi/net.rs:525`, `:647` |
| **`SCM_RIGHTS`** | **complet**, avec `MSG_CTRUNC` | `abi/net.rs:420`, `:466` |
| `eventfd` / `eventfd2` | complet | `abi/file.rs` |

C'est la bonne surprise de cet audit. `SCM_RIGHTS` — le passage d'un descripteur
d'un processus a un autre — est implemente pour de vrai : lecture des `cmsg`
avec l'alignement sur huit octets de `CMSG_NXTHDR`, ecriture bornee au tampon
que l'appelant a dimensionne, et `MSG_CTRUNC` pose quand des descripteurs n'ont
pas tenu (`abi/net.rs:489`). Ce dernier detail n'est pas cosmetique : sans lui,
un recepteur attendrait indefiniment un descripteur qui n'arrivera jamais.

`socketpair` construit deux tampons `Canal` distincts (`a_to_b`, `b_to_a`) pour
un vrai duplex — pas un tube deguise. Un protocole de controle requete/reponse
entre l'interface et le renderer tient dessus sans intermediaire.

## 4. Memoire partagee

| Mecanisme | Etat | Ou |
|---|---|---|
| `memfd_create` | complet, `MFD_CLOEXEC` respecte | `abi/file.rs:505` |
| `ftruncate` sur memfd | complet | `abi/file.rs:1084` |
| `mmap(MAP_SHARED)` sur fichier | **vraies frames partagees** | `abi/mem.rs:142`, `:211` |
| `msync` | complet (recopie globale) | `abi/mem.rs:336` |
| `/dev/shm` | present | `shm-probe.c:246` |

`MAP_SHARED` n'est pas un mensonge ici, et c'est le point qui decide de tout :
les pages viennent d'un **cache global indexe par (nœud, numero de page)**
(`abi/mem.rs:169`), et `map_foreign` les installe dans l'espace du processus
sans les lui donner — elles ne seront donc ni liberees avec lui, ni dupliquees
par `fork`. Deux processus qui mappent le meme `memfd` ecrivent physiquement au
meme endroit.

`shm-probe.c` etablit exactement cette chaine, et rien de plus : `memfd_create`
→ `ftruncate` → `mmap(MAP_SHARED)` → `fork` → le fils ecrit → le pere relit et
doit voir l'ecriture, y compris **au-dela de la premiere page**. Puis la meme
chose a travers un `socketpair` et `SCM_RIGHTS`, entre deux processus qui ne
sont pas parents par le mmap. C'est le scenario d'un tampon d'image passe du
processus qui dessine a celui qui compose.

**Le cycle de vie, ajoute depuis.** Ce document signalait ici une fuite : rien
n'evinçait jamais le cache, et les frames d'un `memfd` ferme restaient allouees
jusqu'a l'extinction. C'est corrige — `kernel/partage.rs` compte trois choses
la ou il n'y en avait aucune : **descripteurs ouverts**, **mappages vivants** et
**possession des pages**. Les frames ne repartent que lorsque les deux premiers
comptes sont a zero, si bien qu'un descripteur ferme dont le mappage vit encore
ne libere rien — ce que POSIX promet. `Drop for Process` couvre la mort brutale :
tuer un renderer ne fait plus fuir ses surfaces. `shm-probe.c` enchaine mille
creations/destructions de surface et compare la memoire physique libre avant et
apres ; il verifie aussi la contre-epreuve, qu'un mappage vivant survive a la
fermeture de son descripteur.

Reste, sans consequence aujourd'hui : le cache est un `Vec` parcouru
lineairement par page mappee. Projeter une surface de 4 MiB, soit 1 024 pages,
coute jusqu'a ~500 000 comparaisons. Correct, mais a revoir si les surfaces
grossissent.

## 5. Synchronisation

`futex` couvre `WAIT`, `WAKE`, `WAIT_BITSET`, `WAKE_BITSET` et rabat
`REQUEUE`/`CMP_REQUEUE` sur un simple reveil — fonctionnellement juste, moins
efficace (`abi/mod.rs:653`). Le piege d'ABI est traite : `FUTEX_WAIT` recoit une
duree relative, `FUTEX_WAIT_BITSET` une **echeance absolue**, et c'est cette
seconde forme qu'emploie `pthread_cond_timedwait`.

Le fait qui compte pour le multi-processus est ailleurs, dans `task.rs` :

```rust
/// Cle d'attente d'un futex : adresse physique du mot surveille, pour que deux
/// threads partageant la page s'accordent meme via des adresses virtuelles
/// differentes.
fn futex_key(uaddr: u64) -> u64 {
    process.space.translate(uaddr).unwrap_or(uaddr)
}
```

La cle est **l'adresse physique**. Un mutex ou une variable de condition posee
dans un `memfd` partage fonctionne donc entre deux processus, qui la voient a
deux adresses virtuelles differentes. C'est la brique sans laquelle une file de
trames en memoire partagee n'aurait aucun moyen de reveiller son consommateur —
et elle est deja la, pour une raison qui n'avait rien a voir.

## 6. Ordonnanceur

**C'est ici que le projet multi-processus perd la moitie de sa promesse.**

- **Un seul CPU.** `sched_getaffinity` rend un masque a 1 (`abi/mod.rs:407`).
  Pas de SMP. Deux processus ne calculent jamais en parallele : ils se
  partagent le meme temps.
- **Pas de priorites POSIX completes.** `sched_setscheduler`, `sched_setparam`
  et `sched_setaffinity` rendent toujours 0 sans rien faire. Ce qui existe
  passe par `setpriority`/`getpriority` et se resume au signe de `nice` : deux
  classes, pas vingt niveaux.
- **Tourniquet simple**, commutation aux points de blocage volontaires et sur
  IRQ0 uniquement si le timer a interrompu du code ring 3 (`task.rs:17-27`). Le
  noyau lui-meme n'est jamais preempte : il n'est pas reentrant.
- **Un seul quota.** `RLIMIT_AS` se pose, se relit et s'applique dans `brk` et
  `mmap` : un processus qui depasse recoit `ENOMEM` et survit a son propre
  depassement. C'est delibere — c'est le seul quota qui change quelque chose au
  projet de processus separes. Rien ne borne en revanche le temps CPU, le
  nombre de descripteurs, ni la taille cumulee des tampons IPC.
- **Deux classes de priorite**, `Interactive` et `Normale`, posees par
  `setpriority`. Le tourniquet sert d'abord les taches interactives pretes, et
  rend la main apres huit tours consecutifs pour qu'aucune tache normale ne soit
  affamee. Ce n'est pas un ordonnanceur different : c'est le meme, avec un ordre
  de service.

La consequence que la premiere version de ce document enoncait — « sortir le
rendu n'empechera pas une page lourde de rendre l'interface lente » — n'est plus
vraie. Elle l'etait tant que le tourniquet servait tout le monde a egalite ;
elle cesse de l'etre des lors qu'une classe passe avant l'autre.

Ce qui reste vrai : un seul cœur veut dire que l'interface passe **avant**, pas
qu'elle tourne **en meme temps**. C'est suffisant pour la reactivite — un reveil
servi a l'heure —, ce ne l'est pas pour le debit. Le SMP repond a la seconde
question, et seulement a elle.

## 7. Attente d'evenements

| Appel | Etat | Ou |
|---|---|---|
| `poll` / `ppoll` | fonctionnel, **par attente active** | `abi/file.rs:1580` |
| `select` / `pselect6` | idem, lecture seule | `abi/file.rs:1622` |
| `epoll_create` / `_ctl` / `_wait` | fonctionnel, niveau uniquement | `abi/file.rs:1661`+ |
| `timerfd_*` | complet | `abi/file.rs` |

`readable()` (`abi/file.rs:1546`) couvre console, fichiers, tubes — y compris le
cas « ecrivain disparu donc pret, la lecture rendra 0 », sans quoi une boucle
`poll` attendant la fin de fichier tournerait sans fin —, clavier et souris
**sans consommer l'evenement**, `eventfd`, `timerfd`, sockets et `socketpair`.
La couverture est bonne.

La forme, elle, ne l'est pas. `sys_poll` est une boucle qui re-teste tous les
descripteurs, puis `yield_now()` (`abi/file.rs:1587-1618`). Il n'y a pas de file
d'attente : une tache bloquee dans `poll` reste `Ready` et se fait reordonnancer
a chaque quantum pour re-tester. Avec un processus, c'est invisible. Avec une
interface et N renderers tous en `poll`, l'ordonnanceur depense ses quanta a
re-tester — un cout en O(processus x descripteurs) par tick.

`POLLOUT` etait **toujours** rendu pret ; ce document le signalait comme le
second defaut a corriger avant tout protocole IPC. C'est fait. Tubes et paires
de sockets ont desormais une capacite (`CAPACITE_CANAL`, 64 KiB) : plein, plus
de `POLLOUT` et `EAGAIN` a l'ecriture non bloquante ; presque plein, une
ecriture courte qui dit combien d'octets sont reellement partis ; lecteur ferme,
`POLLHUP`/`POLLERR` rendus sans qu'on les demande et `EPIPE` a l'ecriture.

`ipc-probe.c` etablit les quatre etats. Il a d'abord ete passe au noyau Linux de
l'hote, qui y a trouve deux erreurs — un `SIGPIPE` non ignore, et l'attente d'un
`POLLOUT` apres une lecture partielle alors que Linux applique un seuil de place
libre. Un test qui n'est vrai que contre son propre noyau ne prouve rien.

`epoll` n'a pas de declenchement sur front (`EPOLLET`) : la liste est relue
entierement a chaque tour.

## 8. Signaux

Complet et soigne (`abi/proc.rs:266-452`, `kernel/signal.rs`) :
`rt_sigaction`, `rt_sigprocmask`, `rt_sigreturn`, `sa_mask`, `SA_NODEFER`,
`SA_RESETHAND`, actions par defaut, `SIGKILL`/`SIGSTOP` ni interceptables ni
blocables — refuses a `rt_sigaction` *et* revalides a la livraison, ceinture et
bretelles. Un signal reveille une tache endormie (`task::wake_for_signal`),
sinon un `SIGTERM` sur un processus bloque resterait lettre morte. `tkill` est
distingue de `kill` parce que c'est par lui que `raise()` de musl s'envoie un
signal a lui-meme — les confondre ferait echouer tout `abort()` et tout
`assert()`.

Un seul signal est livre par retour en ring 3, le suivant au retour du
gestionnaire courant : correct, et suffisant.

## 9. Terminaison forcee

`kill(pid, SIGKILL)` avec `pid > 0` fonctionne : `send_signal` leve le signal,
reveille la cible, et `deliver_pending` la termine par `exit_group(128 + sig)`
sans passer par un gestionnaire (`abi/proc.rs:426`). `wait4` decode le statut en
distinguant mort par signal et code de sortie (`abi/proc.rs:232`).

Tuer un renderer bloque et le recolter fonctionne donc de bout en bout. C'est la
primitive dont depend le « cet onglet ne repond pas » d'un navigateur, et elle
est prete.

---

## Renderer isolation readiness — le verdict

*Mis a jour le 11 aout 2026, apres l'eviction du cache de pages, `POLLOUT`
honnete, `RLIMIT_AS` et les deux classes d'ordonnancement. Le renderer n'existe
toujours pas — ce qui suit dit ce qu'il trouverait.*

Le prototype minimal honnete : **un processus d'interface, un processus de
rendu**. Le renderer construit une liste d'affichage et peint dans une surface
partagee ; l'interface compose et gere les entrees.

### Les cinq axes

| Axe | Etat | Ce qui le porte |
|---|---|---|
| **crash isolation** | **READY** | `fork` + `execve` + `wait4` + `SIGCHLD` de bout en bout ; `kill(pid, SIGKILL)` tue et se recolte. Un renderer qui plante ou qu'on tue laisse l'interface debout. Reserve inchangee : `fork` copie l'espace d'adressage immediatement, sans copie a l'ecriture — il faut donc forker tot, avant que le tas ne grossisse. |
| **memory quota** | **READY** | `RLIMIT_AS` se pose, se relit et s'applique dans `brk` et `mmap`. Un renderer qui fuit meurt seul au lieu d'emporter la machine. C'est le seul quota : ni temps CPU, ni descripteurs, ni tampons IPC cumules. |
| **IPC backpressure** | **READY** | Tubes et paires de sockets bornes a 64 KiB. Plein : pas de `POLLOUT`, `EAGAIN` a l'ecriture non bloquante, ecriture courte quand il reste un peu de place. Lecteur ferme : `POLLHUP`/`POLLERR` sans qu'on les demande, et `EPIPE`. |
| **shared buffers** | **READY** | `memfd_create` + `ftruncate` + `mmap(MAP_SHARED)` sur des frames reellement partagees, transmises par `SCM_RIGHTS`, synchronisees par un `futex` dont la cle est l'**adresse physique**. Le cycle de vie est complet : les frames reviennent quand le dernier descripteur **et** le dernier mappage ont disparu. |
| **scheduler priority** | **READY** | Deux classes, `Interactive` et `Normale`, posees par `setpriority(PRIO_PROCESS, 0, nice)`. Le tourniquet sert d'abord les taches interactives pretes ; au-dela de huit tours consecutifs, il rend la main pour un tour, ce qui empeche la famine. Un programme portable n'a rien de special a faire : `nice(-5)` suffit. |

### Ce qui manque encore

* **Un seul CPU.** `sched_getaffinity` rend toujours un masque a 1. Les
  priorites font que l'interface passe **avant**, pas qu'elle tourne **en meme
  temps**. C'est suffisant pour la reactivite ; ce ne l'est pas pour le debit.
* **`poll` et `epoll_wait` attendent activement.** Une tache bloquee reste
  `Ready` et se fait reordonnancer pour re-tester. Invisible avec un processus,
  a mesurer des qu'il y en aura plusieurs qui attendent chacun sur son canal —
  c'est le premier chiffre a prendre le jour ou le renderer existe.
* **`clone(CLONE_VFORK)` en `ENOSYS`**, donc pas de `posix_spawn`. Contournable
  par `fork` + `execve`.
* **Le cache de pages reste un `Vec` parcouru lineairement**, en O(pages) par
  mappage. A revoir si une surface depasse quelques mebioctets.

### Verdict

**Les cinq axes sont prets.** Le dernier obstacle que cet audit avait
identifie — « la separation n'achetera pas la reactivite » — est leve : deux
classes d'ordonnancement suffisent a faire passer l'interface devant le calcul,
sur un cœur unique et sans remplacer le tourniquet.

Ce qu'un renderer separe achete desormais :

* un onglet qui plante ou qu'on tue laisse l'interface debout — **oui** ;
* un onglet qui fuit meurt seul, sans emporter la machine — **oui** ;
* un onglet qui calcule laisse l'interface fluide — **oui, et c'est desormais
  mesure** (voir ci-dessous).

### La mesure, enfin faite

`ordonnanceur-probe` a ete jouee sous Bouchaud OS, dans QEMU, par
`tools/test.sh`. Le dispositif : un processus se reveille toutes les 16 ms et
note son retard, pendant que **huit** processus calculent sans jamais se
bloquer. Les chiffres, en microsecondes :

| | median | p95 | p99 | pire |
|---|---|---|---|---|
| au repos | 0 | 0 | 0 | 0 |
| sous charge, priorite **normale** | 4 000 | 7 000 | 8 000 | 8 000 |
| sous charge, priorite **interactive** | 0 | 0 | 1 000 | 1 000 |

Travail accompli par les calculs pendant la mesure : **39 055 tours** en
normal, **39 492 tours** en interactif. Aucune famine — ils en font meme
imperceptiblement plus, la difference etant du bruit.

Trois choses a lire dans ce tableau :

* **la degradation sans priorite est exactement celle que le tourniquet
  predit** : un quantum d'un tick par processus pret, soit huit ticks de pire
  cas pour huit concurrents. Le modele et la mesure coincident, ce qui est la
  meilleure raison de croire les deux ;
* **la priorite ramene l'interface a son plancher.** Huit millisecondes de
  retard sur une trame de seize, c'est une demi-trame perdue ; une
  milliseconde ne se voit pas. Le facteur est de huit, et il n'a rien coute au
  calcul ;
* **la resolution de l'horloge est de 1 000 us** — un tick de 1 kHz. C'est
  pour cette raison que la premiere version de la sonde, qui n'employait qu'un
  seul processus de calcul, ne mesurait rien : le pire cas attendu valait alors
  un tick, c'est-a-dire le pas de l'instrument. Elle refusait honnetement de
  conclure ; il fallait augmenter la charge, pas assouplir le critere.

`scheduler priority` passe donc de **READY par construction** a **READY,
mesure**.

## Ordre de travail suggere

Les quatre chantiers que cet audit demandait sont faits : eviction du cache de
pages, `POLLOUT` honnete, `RLIMIT_AS`, priorites d'ordonnancement. La mesure
qui les couronnait l'est aussi. Ce qui vient ensuite :

1. ~~**Le prototype de renderer.**~~ Fait, et depasse : le renderer separe n'est
   plus un prototype mais le chemin par defaut du navigateur, avec ses
   ressources courtees par le processus qui tient la fenetre et sa table de
   descripteurs nettoyee au `fork`. Voir `BROWSER_ISOLATION.md` et
   `RENDERER_PRIVILEGE_AUDIT.md`. La mesure de reactivite ci-dessus a
   maintenant sa version avec un renderer reel comme charge
   (`navigateur/ordonnanceur-navigateur.py`), qui reste a jouer sous QEMU.
2. **Instrumenter l'attente active** de `poll`/`epoll_wait` le jour ou
   plusieurs processus attendent. Si le cout reste negligeable, ne rien faire.
3. **Le SMP**, qui rend la separation *rentable* sans la rendre *possible*.
   Chantier d'un autre ordre, prealable a rien.
