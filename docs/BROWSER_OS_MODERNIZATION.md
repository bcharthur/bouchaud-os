# Ce que le noyau sait deja faire — et ce qui manque au renderer separe

*Etat au 11 aout 2026. Aucun processus de rendu n'existe encore : ce document
mesure le terrain avant d'y construire.*

## Pourquoi ce document

Le navigateur est aujourd'hui **un seul processus** : `/bo-navigateur`, un ELF
statique qui contient Qt, CPython, QuickJS et ffmpeg. Une page qui fait boucler
le moteur fige l'interface ; une page qui epuise la memoire tue l'application
entiere. C'est exactement le probleme que l'architecture multi-processus de
Chromium et de WebKit resout — et la question posee ici est de savoir si ce
noyau peut la porter.

La reponse courte est **oui pour l'isolation des pannes, non pour l'isolation
des ressources**, et la difference compte : separer le renderer sur cet OS
empecherait un site de faire tomber le navigateur, mais **pas** de le rendre
lent. Le reste du document dit pourquoi, avec les lignes de code a l'appui.

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

**Un defaut, precis et trouvable.** Rien n'evince jamais le cache : aucun chemin
du noyau ne retire d'entree de `PAGE_CACHE`. Les frames d'un `memfd` ferme
restent allouees jusqu'a l'extinction de la machine. Un compositeur qui recree
sa surface a chaque redimensionnement fait donc fuir de la memoire physique de
maniere non bornee. Accessoirement, le cache est un `Vec` parcouru lineairement
par page mappee (`abi/mem.rs:188`) : projeter une surface de 4 MiB, soit 1 024
pages, coute jusqu'a ~500 000 comparaisons. Correct, mais quadratique.

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
- **Aucun quota.** `setrlimit` rend 0 sans enregistrer (`abi/mod.rs:496`).
  Aucune borne memoire ni CPU par processus.

Consequence a enoncer clairement, parce qu'elle contredit l'argument habituel :
sur cet OS, sortir le rendu dans un processus separe **n'empeche pas une page
lourde de rendre l'interface lente**. Le renderer et l'interface se disputent le
meme cœur en tourniquet, sans qu'aucun mecanisme ne favorise le second. Et une
page qui fuit emporte toujours la machine, puisque rien ne plafonne sa memoire.

Ce que la separation apporte quand meme, et qui n'est pas rien : un renderer qui
segfault ou qu'on tue laisse l'interface debout, avec un onglet mort au lieu
d'une application morte.

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

Et `POLLOUT` est **toujours** rendu pret (`abi/file.rs:1604`, « les ecritures ne
bloquent jamais ici »). Un renderer qui se sert de `poll` pour savoir quand il
peut ecrire davantage dans un `socketpair` sature recevra un « pret » a chaque
tour et tournera a vide. Tout protocole IPC avec contre-pression bute la-dessus.

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

## La distance jusqu'au prototype multi-processus

Le prototype minimal honnete : **un processus d'interface, un processus de
rendu**. Le renderer construit une liste d'affichage et peint dans une surface
partagee ; l'interface compose et gere les entrees. Voici, brique par brique, ce
qu'il demande et ce qu'il trouve.

| Ce qu'il faut | Etat | Reste a faire |
|---|---|---|
| Lancer le renderer | **pret** — `fork` + `execve` | forker tot (zygote), avant que le tas ne grossisse |
| Canal de controle | **pret** — `socketpair` duplex | protocole a definir cote moteur |
| Passer la surface | **pret** — `memfd` + `SCM_RIGHTS` + `MAP_SHARED` | rien au noyau |
| Reveiller le consommateur | **pret** — `futex` cle par adresse physique | rien au noyau |
| Detecter la mort du renderer | **pret** — `SIGCHLD` + `wait4` | recolter depuis un thread dedie (`wait4` bloque la tache) |
| Tuer un renderer fige | **pret** — `kill(pid, SIGKILL)` | detection du figeage cote interface |
| Contre-pression sur le canal | **partiel** — `POLLOUT` toujours vrai | rendre `poll` honnete en ecriture |
| Ne pas fuir a chaque surface | **manquant** — le cache de pages n'evince jamais | liberer les frames a la fermeture du dernier descripteur |
| Garder l'interface reactive | **manquant** — un CPU, pas de priorites | SMP, ou au minimum des priorites d'ordonnancement |
| Borner un renderer qui fuit | **manquant** — `setrlimit` sans effet | `RLIMIT_AS` reellement applique a `mmap`/`brk` |
| `posix_spawn` | **manquant** — `CLONE_VFORK` en `ENOSYS` | contournable par `fork` + `execve` |

**Verdict.** Le prototype est constructible aujourd'hui : chacune des six
primitives dont il a strictement besoin existe, et cinq d'entre elles sont deja
exercees a l'execution par `shm-probe`. Aucune modification du noyau n'est un
prealable.

Mais il faut savoir ce qu'on achete. La separation donne **l'isolation des
pannes** — un onglet qui meurt ne tue plus le navigateur — et rien d'autre. Elle
ne donne ni la reactivite (un cœur, un tourniquet, pas de priorites) ni la
protection contre l'epuisement memoire (pas de quota). Les vendre ensemble
serait malhonnete, et surtout ce serait construire une architecture pour des
benefices qu'elle n'apportera pas.

## Ordre de travail suggere

Deux chantiers noyau valent d'etre faits **avant** le renderer, parce qu'ils
sont petits et que les decouvrir apres coute cher :

1. **Evincer le cache de pages** a la fermeture du dernier descripteur d'un
   `memfd`. Sans cela, la premiere demonstration qui redimensionne la fenetre
   epuise la memoire physique — et le symptome ne designera pas sa cause.
2. **`POLLOUT` honnete** sur `socketpair` et tubes : rendre pret seulement si le
   tampon a de la place. Un protocole IPC sans contre-pression n'est pas un
   protocole.

Deux autres relevent d'une decision d'architecture, pas d'un correctif :

3. **Des priorites d'ordonnancement** (meme grossieres : deux classes,
   interface et arriere-plan). C'est le seul moyen, sur un cœur unique, de
   rendre la separation utile a la reactivite.
4. **`RLIMIT_AS` applique** dans `sys_mmap` et `sys_brk`. Quelques lignes, et
   c'est ce qui transforme « le renderer a fui » en « le renderer est mort »
   plutot qu'en « la machine est morte ».

Le SMP est un chantier d'un autre ordre, et il n'est pas un prealable : il rend
la separation *rentable*, il ne la rend pas *possible*.
