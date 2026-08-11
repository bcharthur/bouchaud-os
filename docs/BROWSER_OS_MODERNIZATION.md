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

La reponse courte est **oui pour l'isolation des pannes et de la memoire, non
pour celle du temps de calcul**. Separer le renderer sur cet OS empecherait un
site de faire tomber le navigateur, et — depuis que `RLIMIT_AS` est applique —
d'epuiser la memoire de la machine. Cela ne l'empecherait toujours **pas** de le
rendre lent : un seul cœur, un tourniquet, aucune priorite. Le reste du document
dit pourquoi, avec les lignes de code a l'appui.

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
- **Pas de priorites.** `sched_setscheduler`, `sched_setparam` et
  `sched_setaffinity` rendent 0 sans rien faire ; `sched_get_priority_max` rend
  0 (`abi/mod.rs:414`). Un renderer ne peut etre ni privilegie ni relegue.
- **Tourniquet simple**, commutation aux points de blocage volontaires et sur
  IRQ0 uniquement si le timer a interrompu du code ring 3 (`task.rs:17-27`). Le
  noyau lui-meme n'est jamais preempte : il n'est pas reentrant.
- **Un seul quota.** `RLIMIT_AS` se pose, se relit et s'applique dans `brk` et
  `mmap` : un processus qui depasse recoit `ENOMEM` et survit a son propre
  depassement. C'est delibere — c'est le seul quota qui change quelque chose au
  projet de processus separes. Rien ne borne en revanche le temps CPU, le
  nombre de descripteurs, ni la taille cumulee des tampons IPC.

Consequence a enoncer clairement, parce qu'elle contredit l'argument habituel :
sur cet OS, sortir le rendu dans un processus separe **n'empeche pas une page
lourde de rendre l'interface lente**. Le renderer et l'interface se disputent le
meme cœur en tourniquet, sans qu'aucun mecanisme ne favorise le second.

Ce que la separation apporte, en revanche : un renderer qui segfault ou qu'on
tue laisse l'interface debout, avec un onglet mort au lieu d'une application
morte — et, depuis `RLIMIT_AS`, un renderer qui fuit meurt seul au lieu
d'emporter la machine. C'est deux benefices sur trois ; le troisieme demande des
priorites d'ordonnancement, pas une architecture.

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

## Renderer isolation readiness — le verdict, apres les correctifs

*Mis a jour le 11 aout 2026, apres l'eviction du cache de pages, `POLLOUT`
honnete et `RLIMIT_AS`. Le renderer n'existe toujours pas — ce qui suit dit
seulement ce qu'il trouverait s'il etait ecrit.*

Le prototype minimal honnete : **un processus d'interface, un processus de
rendu**. Le renderer construit une liste d'affichage et peint dans une surface
partagee ; l'interface compose et gere les entrees.

### Les cinq axes

| Axe | Etat | Ce qui le porte |
|---|---|---|
| **Memory safety** | **pret** | Le cache de pages compte descripteurs ouverts, mappages vivants et possession des pages ; les frames ne partent que lorsque les deux premiers comptes sont a zero. `Drop for Process` couvre la mort brutale — tuer un renderer ne fait plus fuir ses surfaces. Un mappage vivant survit a la fermeture de son descripteur, ce qui est ce que POSIX promet. |
| **IPC backpressure** | **pret** | Tubes et paires de sockets ont une capacite (64 KiB). Plein : pas de `POLLOUT`, `EAGAIN` a l'ecriture non bloquante, ecriture courte quand il reste un peu de place. Lecteur ferme : `POLLHUP`/`POLLERR` sans qu'on les demande, et `EPIPE`. Un protocole de controle peut enfin ralentir au lieu de gonfler. |
| **Process lifecycle** | **pret** | `fork` + `execve` + `wait4` + `SIGCHLD` de bout en bout ; `kill(pid, SIGKILL)` tue et se recolte. Reserve inchangee : `fork` copie l'espace d'adressage **immediatement**, sans copie a l'ecriture. Il faut donc forker tot — le *zygote* de Chromium — avant que le tas ne grossisse. |
| **Shared surfaces** | **pret** | `memfd_create` + `ftruncate` + `mmap(MAP_SHARED)` sur des frames reellement partagees, transmises par `SCM_RIGHTS`, synchronisees par un `futex` dont la cle est l'**adresse physique** — donc valable entre deux processus qui voient la surface a deux adresses virtuelles differentes. `shm-probe` etablit la chaine entiere, cycle de vie compris. |
| **Resource quota** | **partiel** | `RLIMIT_AS` se pose, se relit et s'applique dans `brk` et `mmap` : un renderer qui fuit meurt seul au lieu d'emporter la machine. Mais c'est le **seul** quota. Rien ne borne le temps CPU, le nombre de descripteurs ni la taille des tampons IPC cumules. |

### Ce qui reste manquant, et ce que cela coute

* **Un seul CPU, pas de priorites.** `sched_getaffinity` rend toujours un
  masque a 1 ; `sched_setscheduler` rend toujours 0 sans rien faire. C'est le
  manque qui decide de la valeur du renderer separe : **la separation
  n'achetera pas la reactivite**. Un renderer qui calcule prend le cœur au
  meme titre que l'interface, et rien ne permet de le reculer.
* **`poll` et `epoll_wait` attendent activement.** Une tache bloquee reste
  `Ready` et se fait reordonnancer pour re-tester. Invisible avec un
  processus ; a surveiller des qu'il y en aura plusieurs qui attendent chacun
  sur son canal. Le cout n'est pas mesure aujourd'hui parce qu'il n'y a rien a
  mesurer — c'est le premier chiffre a prendre le jour ou le renderer existe.
* **`clone(CLONE_VFORK)` en `ENOSYS`**, donc pas de `posix_spawn`. Contournable
  par `fork` + `execve`, qui est le chemin qu'emploie deja `subprocess`.
* **Le cache de pages reste un `Vec` parcouru lineairement.** Correct, mais en
  O(pages) par mappage. A revoir si une surface depasse quelques mebioctets.

### Verdict

**Le prototype est constructible, et il achete maintenant deux choses au lieu
d'une.** Avant les correctifs, separer le rendu donnait l'isolation des pannes
et rien de plus — avec, en prime, une fuite de memoire physique a chaque
surface recreee et un canal IPC incapable de ralentir. Ces deux defauts sont
corriges et eprouves.

Ce qu'on obtiendrait aujourd'hui :

* un onglet qui plante ou qu'on tue laisse l'interface debout ; **oui** ;
* un onglet qui fuit meurt seul, sans emporter la machine ; **oui**, sous
  `RLIMIT_AS` ;
* un onglet qui calcule laisse l'interface fluide ; **non**, et cela ne
  changera pas sans priorites d'ordonnancement.

La troisieme ligne est celle qui doit decider du calendrier. Tant qu'elle est
fausse, le renderer separe est un chantier lourd pour un benefice reel mais
partiel — et le moteur, lui, a encore des gains a prendre sans changer
d'architecture.

## Ordre de travail suggere

Les deux correctifs qui devaient preceder le renderer sont faits :
l'eviction du cache de pages et `POLLOUT` honnete. `RLIMIT_AS` s'y est ajoute
parce qu'il s'est revele reellement local — quelques lignes dans `sys_mmap` et
`sys_brk`, et « le renderer a fui » cesse d'etre « la machine est morte ».

Ce qui vient ensuite, dans l'ordre du rapport qualite/prix :

1. **Des priorites d'ordonnancement minimales** — deux classes suffiraient :
   interface et arriere-plan. C'est **la** prochaine amelioration rentable de
   l'ordonnanceur, et elle l'est bien avant le SMP : sur un cœur unique, c'est
   le seul moyen de rendre la separation utile a la reactivite, et cela se
   greffe sur le tourniquet existant sans le remplacer. Un `nice` honnete
   plutot qu'un ordonnanceur entierement different.
2. **Instrumenter l'attente active** de `poll`/`epoll_wait` le jour ou
   plusieurs processus attendent. Si le cout reste negligeable, ne rien faire ;
   s'il grimpe, remplacer la boucle par de vraies files d'attente devient le
   chantier OS prioritaire.
3. **Le SMP**, qui rend la separation *rentable* sans la rendre *possible*.
   C'est un chantier d'un autre ordre, et il n'est un prealable a rien.

