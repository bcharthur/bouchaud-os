# Userland de Bouchaud OS

Le noyau expose désormais l'ABI Linux x86-64 : un binaire compilé pour Linux
s'exécute en ring 3 sans recompilation du noyau. Ce dossier contient la chaîne
de construction côté utilisateur.

```
./build.sh freestanding    # tests d'ABI sans libc (gcc + ld, aucune dépendance)
./build.sh musl            # binaires statiques musl (dont qpa-probe)
./build.sh musl-dynamic    # binaires dynamiques + ld-musl-x86_64.so.1
./build-python.sh          # CPython 3.12 statique + bibliothèque standard  (§11)
./build-qt.sh              # Qt 5.15 statique + démonstration linuxfb        (§9)
```

Les binaires produits vont dans `out/`. On les installe sur la machine en
fabriquant l'image du disque de données :

```
./mkdisk.sh              # archive out/ dans userland.img
```

`run.ps1` et `boot.ps1` attachent automatiquement cette image comme second
disque si elle existe ; le noyau la déplie dans le RAMFS au démarrage. **Il n'y
a plus besoin de recompiler l'OS pour y installer un programme.**

Puis, dans le shell :

```
exec /hello
elfinfo /hello        # type ELF, segments, interpréteur requis
tasks                 # threads du programme en cours
syscalls              # les 107 appels implémentés, par famille
strace on             # trace des appels système sur COM1
```

## Vérification automatique

```
./tools/test.sh
```

Construit le noyau et les sondes, fabrique le disque, démarre QEMU sans
affichage, analyse le journal série et renvoie un code de retour. Le noyau joue
le fichier `/autorun` déposé sur le disque au lieu d'ouvrir une session, puis
éteint la machine en signalant son verdict à l'hôte (voir
`src/kernel/autorun.rs`).

Si `out-python/` et `out-qt/` existent, la sonde Python et la démonstration Qt
sont ajoutées au scénario.

## Comment les fichiers arrivent sur la machine

Le noyau lit le second disque au démarrage, y cherche une archive `tar` et la
déplie dans le RAMFS. C'est le principe d'un `initramfs`.

Ce détour n'est pas gratuit : jusqu'ici, déposer un programme imposait de
l'inclure dans le noyau par `include_bytes!` puis de tout recompiler. Avec une
image de boot déjà supérieure à 20 Mio, cela rendait matériellement impossible
d'installer une pile logicielle réelle — et imposait une reconstruction
complète de l'OS à chaque itération.

Détails d'implémentation :

* **Pilote ATA en PIO** (`src/drivers/ata.rs`), pas virtio-blk. Un virtio
  serait plus rapide mais réclame une file de descripteurs, des tampons DMA et
  une négociation de fonctionnalités — beaucoup de surface pour « lire une
  archive au démarrage ». Le transfert utilise `rep insw`, qui déplace un
  secteur par instruction : c'est ce qui rend le PIO utilisable pour des
  dizaines de mégaoctets sous émulation.
* **Format `tar` ustar** (`src/fs/tar.rs`), pas un système de fichiers. La
  lecture est séquentielle et unique ; un FAT ou un ext2 demanderait un
  allocateur de blocs et une table d'inodes pour un besoin qui n'en a pas.
  L'écriture persistante viendra avec un vrai système de fichiers — ce n'est
  pas ce qui bloquait.
* Les limites du RAMFS ont été relevées en conséquence : **4096 inodes** et
  **64 Mio par fichier** (contre 1024 et 4 Mio). L'ancienne limite de 4 Mio
  interdisait purement et simplement le dépôt d'un binaire lié statiquement.

Vérifié sous QEMU : archive de 11,4 Mio contenant sept fichiers dont un de
10 Mio, dépliée au boot, `exec /hello` et la sonde POSIX complète exécutés
depuis le disque sans aucune recompilation du noyau.

## Contrainte d'adressage (à lire avant de compiler)

Le noyau réserve à l'espace utilisateur **un créneau PML4 de 512 Gio à partir de
`0x4000_0000_0000`**. Les créneaux bas appartiennent au noyau : un `ET_EXEC` lié
à l'adresse Linux habituelle (`0x400000`) partagerait ses tables de pages avec
le noyau, et le chargeur le refuse explicitement plutôt que de corrompre le
système.

Deux façons de s'y conformer :

| Cas | Option de compilation |
|---|---|
| **PIE / static-PIE** (recommandé) | `-static-pie` ou `-fPIE -pie` — aucune adresse fixe, le noyau charge à `0x400000400000` |
| ELF non-PIE | `-mcmodel=large -Wl,-Ttext-segment=0x400000400000` |

`vmstat` affiche les adresses effectives (base de chargement, base `ld.so`, base
`mmap`, sommet de pile).

## État des couches

| Couche | État | Où |
|---|---|---|
| 1. Mémoire virtuelle | frames physiques + espace d'adressage par processus | `src/kernel/vmm.rs` |
| 2. Ring 3 / TSS / syscall | GDT complète, TSS RSP0, `syscall`/`sysretq`, `iretq` | `src/arch/x86_64/{gdt,usermode}.rs` |
| 3. Chargeur ELF64 | `PT_LOAD`, `PT_INTERP`, `PT_TLS`, auxv | `src/kernel/elf.rs` |
| 4. Appels POSIX | 107 appels, numéros et structures Linux | `src/kernel/abi/` |
| 5. Processus / threads | `fork`, `execve`, `wait4`, `clone`, futex, signaux | `src/kernel/{task,signal}.rs` |
| 6. libc musl | **côté utilisateur** — voir ci-dessous | ce dossier |
| 7. `ld.so` | chargé par le noyau, résout en ring 3 | `src/kernel/exec.rs` |
| 8. Runtime C++ | **côté utilisateur** — voir ci-dessous | ce dossier |
| 9. Serveur graphique | `/dev/fb0` mmap + ioctls fbdev + evdev | `src/kernel/{fd,input}.rs` |
| 10. Réseau | sockets TCP/UDP POSIX sur la pile du noyau | `src/kernel/abi/net.rs` |

Les couches 6 et 8 ne sont pas du code noyau : ce sont des bibliothèques à
compiler avec la chaîne ci-dessus. Le noyau n'a rien à savoir de leur contenu,
seulement à honorer les appels système qu'elles émettent — ce qu'il fait.

## 6. libc musl statique

```sh
sudo apt install musl-tools        # fournit musl-gcc
./build.sh musl
```

Ce qu'un `musl` statique exige du noyau, et qui est fourni :

* le vecteur auxiliaire complet (`AT_PHDR`, `AT_PHNUM`, `AT_PHENT`,
  `AT_PAGESZ`, `AT_ENTRY`, `AT_RANDOM`, `AT_HWCAP`, `AT_CLKTCK`) — sans
  `AT_RANDOM`, le canari de pile fait planter le programme avant `main` ;
* `set_tid_address`, `arch_prctl(ARCH_SET_FS)` pour le TLS ;
* `brk` + `mmap`/`mprotect`/`munmap` pour `mallocng` ;
* `writev` (tous les `printf` passent par là), `readv`, `ioctl(TCGETS)` pour
  décider si `stdout` est un terminal ;
* `futex`, `clone`, `set_robust_list` pour `pthread`.

## 7. Éditeur de liens dynamique

Le noyau lit `PT_INTERP`, charge l'interpréteur à une base séparée, et lui donne
la main en lui passant `AT_BASE`/`AT_ENTRY`. Aucune résolution de symbole n'est
faite en ring 0 : `ld.so` mappe les bibliothèques par `mmap` puis saute dans le
programme, exactement comme sous Linux.

Il faut donc placer l'interpréteur dans le RAMFS **au chemin exact** inscrit
dans le binaire (`elfinfo` l'affiche, typiquement `/lib/ld-musl-x86_64.so.1`),
ainsi que les `.so` cherchés dans `LD_LIBRARY_PATH` (`/lib:/usr/lib` par
défaut).

`mmap` de bibliothèque se fait en `MAP_PRIVATE`, donc par copie : deux
processus chargeant la même `.so` ne partagent pas ses pages. C'est correct
(`MAP_PRIVATE` le veut ainsi) mais plus coûteux en mémoire qu'un cache
partagé en lecture seule. Un `MAP_SHARED`, lui, partage réellement les frames
(voir §10).

## 8. Runtime C++

```sh
# libstdc++ liée statiquement, sans dépendance dynamique
musl-g++ -static-pie -O2 -fno-exceptions -fno-rtti app.cpp -o app   # minimal
musl-g++ -static-pie -O2 app.cpp -o app                             # complet
```

Le noyau fournit ce dont le runtime a besoin :
`mmap`/`mprotect` (l'unwinder rend exécutables ses trampolines), `futex`
(`std::mutex`, initialisation des statiques locales), `clone` (`std::thread`),
`arch_prctl` (`thread_local`), et `readlink("/proc/self/exe")` que certaines
implémentations interrogent au démarrage.

Les exceptions fonctionnent avec un `libgcc_eh` statique : le déroulement lit
`.eh_frame` dans l'image chargée, sans intervention du noyau.

## 9. Serveur graphique / Qt

Le noyau expose la surface Linux qu'attend le plugin `linuxfb` de Qt, avec ses
plugins d'entrée `evdevkeyboard` et `evdevmouse`.

| Chemin | Rôle |
|---|---|
| `/dev/fb0` | framebuffer 1280x720 XRGB8888, `mmap`-able sur la VRAM réelle |
| `/dev/tty0` | console virtuelle : bascule en mode graphique (`KD_GRAPHICS`) |
| `/dev/input/event0` | clavier, **codes de touches Linux**, format evdev |
| `/dev/input/event1` | souris, événements **relatifs** `REL_X`/`REL_Y` + boutons |

Ioctls implémentés :

* framebuffer — `FBIOGET_VSCREENINFO`, `FBIOGET_FSCREENINFO`,
  `FBIOPUT_VSCREENINFO`, `FBIOPAN_DISPLAY`, `FBIOBLANK` ;
* console — `KDGETMODE`, `KDSETMODE`, `KDGKBMODE`, `KDSKBMODE`, `VT_GETSTATE`,
  `VT_GETMODE`, `VT_SETMODE`, `VT_ACTIVATE`, `TCGETS`, `TIOCGWINSZ` ;
* evdev — `EVIOCGVERSION`, `EVIOCGID`, `EVIOCGNAME`, `EVIOCGPHYS`, `EVIOCGUNIQ`,
  `EVIOCGPROP`, `EVIOCGBIT(type)`, `EVIOCGKEY`, `EVIOCGLED`, `EVIOCGRAB`,
  `EVIOCSCLOCKID`.

Ouvrir `/dev/fb0` bascule automatiquement la carte en mode graphique ; ouvrir
`/dev/input/event1` arme l'IRQ de la souris. Le mode texte est rendu au shell
quand le programme se termine.

### Deux points à connaître sur les entrées

**Le clavier émet des positions physiques, pas des caractères.** Les codes sont
ceux de Linux (`KEY_A` = 30), ce qui est la seule chose qu'un client evdev sait
interpréter. Le noyau applique un AZERTY-FR pour son propre shell, mais Qt
applique sa disposition à lui — `us` par défaut. Pour retrouver l'AZERTY, il
faut lui passer un keymap :
`QT_QPA_EVDEV_KEYBOARD_PARAMETERS=/dev/input/event0:keymap=fr.qmap`.

**La souris est relative.** Le pilote PS/2 maintient une position absolue pour
le bureau du noyau ; la couche evdev en dérive des deltas, car c'est ainsi
qu'un client distingue une souris d'un écran tactile.

### Boucle d'événements

`QEventDispatcherUNIX` repose sur trois primitives, toutes présentes :
`eventfd` (compteur de réveils, pas un tube — la distinction compte : les
réveils doivent fusionner), `poll`/`ppoll`, et `timerfd`. Le tick noyau est à
**1000 Hz** : sans cela, la granularité de 55 ms du PIT par défaut rendrait
toute animation saccadée et tout `poll` imprécis.

### Vérification

`qpa-probe.c` rejoue la séquence de démarrage complète — mêmes appels, même
ordre, mêmes structures — et vérifie 47 points : ouverture de la console et
bascule en `KD_GRAPHICS`, géométrie et format du framebuffer, `mmap` de la
VRAM, capacités evdev des deux périphériques, boucle `poll` réveillée par un
`eventfd` depuis un autre thread, `timerfd` à 120 ms, `pthread_cond_timedwait`
à échéance absolue, présence des polices et de `/proc`, `/sys`.

```
exec /qpa-probe
```

`fb-demo.c` fait le même travail sur le seul framebuffer, sans aucune
bibliothèque. `posix-probe.c` couvre les processus, les signaux, la mémoire
partagée et le réseau (§10).

### Environnement fourni par défaut

Voir `kernel::exec::default_environment` :

```
QT_QPA_PLATFORM=linuxfb
QT_QPA_PLATFORM_PLUGIN_ARGS=fb=/dev/fb0:size=1280x720
QT_QPA_FB_TTY=/dev/tty0
QT_QPA_FONTDIR=/usr/share/fonts/truetype/dejavu
QT_QPA_EVDEV_KEYBOARD_PARAMETERS=/dev/input/event0:grab=0
QT_QPA_EVDEV_MOUSE_PARAMETERS=/dev/input/event1:grab=0
QT_NO_FONTCONFIG=1   DBUS_SESSION_BUS_ADDRESS=disabled:
SDL_VIDEODRIVER=fbcon   SDL_FBDEV=/dev/fb0
```

Le noyau installe aussi au boot (`kernel::sysroot`) les polices DejaVu dans
`/usr/share/fonts/truetype/dejavu`, un `/proc` et un `/sys` réduits aux
fichiers réellement lus au démarrage (`/sys/devices/system/cpu/online` pour
`sysconf(_SC_NPROCESSORS_ONLN)`, `/proc/meminfo`, `/proc/cpuinfo`), et un
`/etc` minimal.

### Compiler Qt

```sh
./build-qt.sh            # télécharge qtbase 5.15, construit Qt statique + la démo
./mkdisk.sh out-qt       # fabrique le disque
# puis, sous l'OS :
exec /qt-demo
```

Une vingtaine de minutes sur quatre cœurs. Le résultat est un binaire
statique-PIE d'environ 16 Mo qui embarque Qt Core, Gui, Widgets et le plugin de
plateforme linuxfb, et qui dessine une vraie fenêtre sur `/dev/fb0`.

Trois choix méritent une explication, tous dans l'en-tête de `build-qt.sh` :

- **glibc et non musl.** Qt est du C++ et réclame une `libstdc++` ; la chaîne
  `musl-tools` n'en fournit pas. Le `g++` du système en apporte une complète,
  exceptions comprises. L'OS s'en moque : il implémente l'ABI de Linux, pas
  celle d'une libc en particulier — ce qui a été vérifié avec un binaire C++
  glibc statique avant de s'engager dans cette voie.
- **statique.** Un exécutable statique ne peut pas charger de `.so`, donc pas
  de plugin à découvrir : le plugin linuxfb est lié en dur par `Q_IMPORT_PLUGIN`
  dans `qt-demo.cpp`. C'est le mode nominal de Qt sur un système embarqué.
- **`-no-glib`.** Sans cela, Qt utilise le dispatcher GLib, qui demande
  davantage au système que sa propre boucle `poll`.

La source Ubuntu (`+dfsg`) est expurgée de ses bibliothèques tierces
embarquées : freetype, libpng, pcre2 et zlib doivent venir du système, en
version statique (`libfreetype-dev libpng-dev libpcre2-dev zlib1g-dev
libbrotli-dev libbz2-dev`).

Qt affiche au démarrage deux avertissements `iconv_open failed`. Ils sont sans
conséquence : la glibc statique n'embarque pas ses modules de conversion
`gconv`, et Qt retombe sur son codec interne.

### Ce qui manque encore

1. **`listen`/`accept`** — pas de socket serveur. Voir §10 ;
2. **IPv6** — `socket(AF_INET6)` répond `EAFNOSUPPORT`, ce qui fait
   correctement retomber `getaddrinfo` sur IPv4 ;
3. **accélération graphique** — tout le rendu est logiciel. Une pile Qt tourne,
   mais au rythme d'un rendu logiciel en 1280x720 ;
4. **QtWebEngine** — hors de portée : c'est Chromium, plusieurs gigaoctets de
   sources et une couche GPU. Un navigateur en Qt sur cet OS passerait par un
   moteur léger, pas par WebEngine.

## 11. Python

```sh
./build-python.sh          # construit CPython 3.12 statique-PIE (musl)
./mkdisk.sh out-python     # fabrique le disque
# puis, sous l'OS :
exec /usr/bin/python3 /mon-script.py
```

L'interprète fait environ 10 Mo, la bibliothèque standard 2,5 Mo. Deux fichiers
en tout : `/usr/bin/python3` et `/usr/lib/python312.zip`.

La bibliothèque standard est livrée en archive zip et non en 2000 fichiers :
c'est ce que `zipimport` sait lire nativement, et c'est autant de nœuds que le
RAMFS n'a pas à créer au démarrage. CPython la cherche tout seul à
`<préfixe>/lib/python312.zip` ; `PYTHONHOME=/usr` fait partie de
l'environnement fourni par défaut.

Toutes les extensions C sont liées dans le binaire (`Modules/Setup.local`) : un
exécutable statique ne peut pas charger de `.so`. Les modules qui réclament une
bibliothèque externe (`_ssl`, `_sqlite3`, `_ctypes`, `_lzma`…) sont désactivés ;
`zlib` est reconstruit contre musl parce que `zipimport` en a besoin.

`python-probe.py` vérifie l'OS depuis l'interprète — fichiers, `fork`, threads,
horloges, `select`/`poll`, sockets, `getrandom`. C'est l'inverse d'une sonde
écrite à la main : on laisse CPython utiliser le système comme il en a
l'habitude, et on regarde ce qui casse. Elle a trouvé trois défauts réels dès sa
première exécution.

## 10. Processus, signaux et réseau

Ces trois couches sont désormais complètes côté noyau. `posix-probe.c` les
vérifie toutes (0 échec sous QEMU).

### Processus

`fork`, `vfork`, `execve`, `wait4`/`waitpid`, filiation parent/enfant, zombies
et récolte du code de sortie. Les descripteurs sont dupliqués par `fork` en
partageant les objets sous-jacents — c'est ce qui fait marcher `cmd1 | cmd2`.

La copie d'espace d'adressage est **immédiate**, pas en copie-à-l'écriture.
C'est plus coûteux au moment du `fork` (le cas `fork` + `execve` paie une copie
inutile), mais cela évite le comptage de références sur chaque frame et le
traitement des fautes d'écriture, deux sources de corruption silencieuse. Le
compromis est assumé, pas subi.

### Signaux

Livraison réelle à un gestionnaire ring 3 : le noyau écrit une trame
(`pretcode`, `ucontext`, `siginfo`) sur la pile utilisateur, détourne
l'exécution vers le gestionnaire, et `rt_sigreturn` restaure l'état exact.
`sigaction`, `sigprocmask`, `SIG_IGN`, `SIG_DFL`, masquage pendant le
gestionnaire, `SA_RESETHAND`, `SA_NODEFER`, `kill`/`tkill`/`tgkill`,
`sigsuspend`, `pause`, `alarm`/`setitimer` (`SIGALRM`), `SIGCHLD` à la mort
d'un fils.

Un point à connaître : la livraison a lieu **au retour d'un appel système**,
seul moment où la trame ring 3 est modifiable. Une tâche qui calcule sans rien
demander au noyau reçoit donc son signal à son prochain appel système. En
pratique tout programme en émet constamment ; le seul cas non couvert serait
une boucle de calcul pur, pour laquelle Linux lui-même ne garantit aucun délai.

### Réseau

`socket`, `connect`, `bind`, `send`/`sendto`/`sendmsg`,
`recv`/`recvfrom`/`recvmsg`, `shutdown`, `getsockname`/`getpeername`,
`setsockopt`/`getsockopt`, `socketpair`, en TCP et en UDP, au-dessus de la pile
de `src/net/`.

C'est suffisant pour que `getaddrinfo` résolve un nom (la libc parle
elle-même au serveur DNS, en UDP — elle n'appelle pas le résolveur du noyau)
et pour qu'un client HTTP fonctionne. Vérifié : `example.com` résolu puis
requête `GET /` avec réponse HTTP lue depuis le ring 3.

Deux formes d'appel comptent, et l'une est facile à oublier : musl émet
`recvmsg`, pas `recvfrom`, dans son résolveur. Ne fournir que `recvfrom` fait
échouer toute résolution de nom alors que `sendto` fonctionne.

**Pas de socket serveur.** `listen`/`accept` répondent `ENOSYS`. La pile est
pilotée par interrogation depuis le contexte appelant : rien ne reçoit de
paquet tant qu'aucun socket ne lit. Un serveur d'écoute réclamerait d'abord une
réception en tâche de fond, puis un demultiplexage par port et une file de
connexions en attente. C'est signalé franchement plutôt que simulé.

### Mémoire partagée

`mmap(MAP_SHARED)` sur fichier passe par un cache de pages global indexé par
(fichier, numéro de page) : deux processus qui mappent le même fichier
pointent sur les **mêmes frames physiques**, et `msync` répercute les écritures
dans le contenu du fichier. `MAP_PRIVATE` reste une copie privée, comme il se
doit.

## Tests d'ABI sans libc

`ring3-selftest.c` vérifie, sans dépendance : `uname`, `getpid`/`gettid`, `brk`,
`mmap`/`mprotect`, TLS par `arch_prctl`, `clock_gettime`, `clone` d'un thread,
`futex` (réveil croisé), `openat`/`read`/`write`/`lseek`, `writev`,
`exit_group`.

Version embarquée dans le noyau, sans aucun fichier : la commande `usermode`
génère un ELF64 en mémoire et l'exécute. C'est le moyen le plus rapide de
vérifier que le ring 3 est fonctionnel après une modification du noyau.

## 12. Navigateur

```sh
./build-qt.sh                                   # Qt statique (§9)
LIBC=glibc OUT=out-python-embed ./build-python.sh   # libpython pour embarquer
./build-navigateur.sh                           # l'assemble
./mkdisk.sh out-navigateur
# puis, sous l'OS :
exec /bo-navigateur                # page d'accueil
exec /bo-navigateur http://…       # une adresse directement
```

Un binaire de 32 Mo qui contient Qt, CPython et le moteur. Il remplace Nautile,
qui vivait dans le noyau — un moteur web n'a rien à faire en ring 0, où une page
mal formée a le même pouvoir qu'un pilote.

### Architecture

```
hote.cpp        Qt : fenêtre, framebuffer, entrées, peinture     (C++)
   ↕ module `bo`
navigateur.py   chrome, historique, événements                   (Python)
moteur/         html · css · mise_en_page · peinture · reseau     (Python)
```

Qt appelle Python, jamais l'inverse pendant la peinture : `paintEvent` demande
une **liste d'affichage** — des tuples plats (`rect`, `texte`, `ligne`…) — et la
peint. Le moteur ne touche jamais un objet Qt, ce qui permet de le tester sans
écran.

**Pas de PyQt.** PyQt expose les 200 000 lignes d'API de Qt à Python ; un
navigateur en utilise une poignée : ouvrir une fenêtre, peindre, mesurer du
texte, recevoir des touches. Le module `bo` fait exactement cela en quelques
centaines de lignes, se construit en dix secondes, et n'a pas besoin d'un PyQt
statique — chose qui n'existe pas vraiment.

**Une seule libc.** Qt est du C++ et tire la `libstdc++` du système, donc la
glibc ; Python doit donc être construit en glibc lui aussi (`LIBC=glibc`). Deux
libc ne cohabitent pas dans un même binaire.

### Ce que le moteur sait faire

Analyse HTML tolérante (balises non fermées, imbrications interdites, attributs
sans guillemets, entités) · sélecteurs CSS de balise, classe, identifiant et
descendance, avec spécificité, cascade et héritage · feuille de l'agent
utilisateur · modèle de boîte complet · mise en page bloc et en ligne avec
retour à la ligne mesuré sur la vraie fonte · listes, texte préformaté ·
HTTP et HTTPS avec redirections et jeux de caractères · `file://` · historique
avant/arrière, liens cliquables, défilement.

### Ce qu'il ne sait pas faire

**JavaScript** — les pages qui se construisent elles-mêmes s'affichent vides.
**Images** — remplacées par leur texte de remplacement. **Flexbox et grid** —
ramenés à un empilement vertical.

### Résolution de noms

`socket.getaddrinfo` ne fonctionne pas dans un binaire glibc statique : la glibc
y délègue à ses modules NSS, qui sont des bibliothèques partagées chargées par
`dlopen`. Le navigateur porte donc son propre client DNS (`moteur/reseau.py`) —
une requête A en UDP vers le serveur de `/etc/resolv.conf`.

La requête part **avant** que le délai d'attente soit posé sur la prise : une
prise déjà passée en non bloquant fait échouer la première émission vers un hôte
dont l'adresse matérielle n'est pas encore connue, et le noyau rend alors
`ENETUNREACH`. C'est un défaut côté noyau, noté dans la feuille de route.

## 13. pywebview — le tuto, tel quel

```sh
./build-navigateur.sh            # embarque pywebview et son moteur Bouchaud OS
./mkdisk.sh out-navigateur
# puis, sous l'OS :
exec /bo-navigateur /usr/share/bo-navigateur/exemple-webview.py
```

Le code du tutoriel n'est pas adapté : c'est l'API publique de pywebview,
inchangée.

```python
import webview

webview.create_window('Bonjour', html='<h1>…</h1>')
webview.start()
```

### Comment ça marche

pywebview est une bibliothèque **à moteurs enfichables** : `webview/platforms/`
contient un module par système d'affichage (Qt/QtWebEngine, GTK/WebKit, Cocoa,
EdgeChromium…) et `guilib.initialize()` retient celui qui se charge. Bouchaud OS
en est un de plus — `webview/platforms/bouchaud.py`, qui rend les pages avec le
moteur natif (§12) sur la toile Qt de l'hôte.

`greffe-pywebview.sh` télécharge pywebview et ses trois dépendances Python pures
(`bottle`, `proxy_tools`, `typing_extensions`), y installe le moteur, et applique
**deux modifications**, les seules :

| Fichier | Modification | Pourquoi |
|---|---|---|
| `webview/guilib.py` | ajoute `bouchaud` aux moteurs connus, essayé en premier sous Linux | la liste des moteurs est fermée dans la bibliothèque |
| `webview/util.py` | `bo:` est classé comme distant | sinon pywebview démarre son serveur HTTP interne pour servir la page, ce qui demande `listen`/`accept` |

### Ce qui marche

Création de fenêtre, `load_html`, titre, taille, écrans, événements de cycle de
vie (`shown`, `loaded`, `closed`), navigation par liens et défilement à la
souris et au clavier, et la fonction passée à `webview.start()` — qui tourne
bien dans son propre fil et peut lire l'état de la fenêtre.

### Ce qui ne marche pas

- **`evaluate_js` et `window.pywebview.api`** — les deux reposent sur
  l'exécution de JavaScript dans la page, que le moteur natif ne fait pas. Le
  moteur lève une exception explicite plutôt que de rendre `None` en silence.
- **Les applications servant des fichiers locaux** (`create_window(url='index.html')`)
  — pywebview les sert par un serveur HTTP interne, qui a besoin de `listen` et
  `accept`. Le noyau ne les implémente pas encore ; c'est le prochain manque à
  combler pour cette pile.
- **`load_url` appelé depuis le fil de `webview.start()`** — défaut ouvert. Le
  travail est bien mis en file pour le fil principal, mais le chargement
  n'aboutit pas : l'émulateur reste à ~56 % de CPU sans que la page change, et
  ce pendant vingt-cinq minutes — ce n'est pas de la lenteur, c'est une boucle.
  Appelé depuis le fil principal — c'est-à-dire au chargement initial et sur un
  clic de lien — `load_url` fonctionne. Voir la feuille de route.
- **Plusieurs fenêtres à l'écran en même temps** — le framebuffer n'a pas de
  gestionnaire de fenêtres. Les fenêtres suivantes sont créées et pilotables,
  mais s'affichent l'une après l'autre dans la même surface.
