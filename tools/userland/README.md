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
./build-quickjs.sh         # QuickJS statique : le moteur JavaScript         (§12)
./build-ffmpeg.sh          # FFmpeg statique : H.264, VP9, AAC, Opus         (§13)
./build-navigateur.sh      # le navigateur : Qt + CPython + QuickJS en un binaire
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

Si `out-python/`, `out-qt/` et `out-navigateur/` existent, la sonde Python, la
démonstration Qt et les vérifications du moteur web sont ajoutées au scénario.

Le moteur web se vérifie aussi **sans l'OS**, en quelques secondes :

```
./test-moteur.sh
```

Le module `bo` — d'ordinaire fourni par l'hôte Qt — y est remplacé par un
bouchon : la mesure du texte devient une règle de trois, le décodage d'image se
limite à lire l'en-tête PNG. Tout le reste du moteur est le vrai, JavaScript
compris. Reconstruire le navigateur et démarrer l'émulateur prend plusieurs
minutes ; ce script quelques secondes, et c'est ce qui en fait un filet utile
pendant qu'on écrit. Le même fichier (`navigateur/test_moteur.py`) tourne sous
l'OS avec le vrai hôte — c'est là qu'il devient une preuve.

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

# ou, depuis le bureau : menu Démarrer → « Navigateur », ou l'icône du même nom.
# Le bureau quitte le mode graphique, rend l'écran au binaire, et le reprend
# quand celui-ci se termine — deux surfaces ne partagent pas le framebuffer.
```

Un binaire de 32 Mo qui contient Qt, CPython et le moteur. Il remplace Nautile,
qui vivait dans le noyau — un moteur web n'a rien à faire en ring 0, où une page
mal formée a le même pouvoir qu'un pilote.

### Architecture

```
hote.cpp        Qt : fenêtre, framebuffer, entrées, peinture     (C++)
   ↕ module `bo`
navigateur.py   chrome, historique, événements                   (Python)
moteur/         html · css · flex · grille · mise_en_page · peinture
                images · js · media · reseau · youtube            (Python)
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
sans guillemets, entités) · sélecteurs CSS de balise, classe, identifiant,
attribut (six opérateurs, drapeau `i`), descendance, enfant direct et **frères
`+` / `~`**, avec spécificité, cascade et héritage · **pseudo-classes
structurelles** (`:nth-child()` et sa famille, `:first-child`, `:empty`…) et
**fonctionnelles** (`:is()`, `:where()`, `:not()`, `:has()`) — un seul moteur de
sélecteurs sert la cascade et `querySelector`, donc les deux répondent la même
chose ·
feuille de l'agent utilisateur · modèle de boîte complet, `box-sizing`, bornes
`min-*`/`max-*`, `calc()`, unités de fenêtre · règles `@media` évaluées contre
la taille réelle de la fenêtre · **propriétés personnalisées** (`--x` / `var()`,
avec valeur de secours) · `:root` et le style de `<html>`, dont `rem` tire sa
référence · pseudo-éléments `::before`/`::after`, `attr()` compris · mise en page bloc et en ligne avec retour à la ligne mesuré sur la
vraie fonte · **disposition flexible** (base, `grow`/`shrink`, `wrap`,
`justify-content`, `align-items`, colonnes) · **grille** (`repeat()`,
`minmax()`, `fr`, `gap`, placement automatique et explicite) ·
`position: absolute`/`fixed`/**`sticky`** avec `top`/`right`/`bottom`/`left` ·
**`order`** en flexbox et **zones nommées** de grille (`grid-template-areas`) ·
**`transform`** (`translate`, `scale`, `rotate`, `skew`, `matrix`) appliqué à la
peinture, zones de liens comprises · **coins arrondis**, **ombres portées et
intérieures**, **dégradés linéaires et radiaux**, **opacité**, et **bords par
côté** — le `border-bottom: 1px solid #eee` qui sépare la moitié des pages du
web · **`object-fit`** et **`aspect-ratio`** · `@layer`, `@supports` et
`@container` traversés plutôt que sautés ·
**pseudo-classes d'état réelles** — `:hover`, `:active`, `:focus` suivent le
pointeur, et la lignée entière est survolée, ce qui tient un menu déroulant
ouvert · **`@keyframes` et `animation`**, **`transition`**, avec les rythmes
`ease*`, `cubic-bezier()` et `steps()` — longueurs, couleurs et transformations
interpolées · **DOM d'ombre isolé** : les règles de la page s'arrêtent à la
frontière, `:host` la traverse vers l'hôte, et les `<slot>` distribuent le
contenu clair, par nom s'il y en a ·
`overflow: hidden` réellement rogné · listes, texte préformaté · **JavaScript**
(QuickJS : DOM, événements à trois phases, minuteries, promesses, XHR/fetch) ·
`getComputedStyle` **résolu** — cascade, héritage et style en ligne réunis ·
`MutationObserver`, `IntersectionObserver`, `ResizeObserver` ·
**composants** (`customElements.define`, cycle de vie complet, `attachShadow`) ·
**canvas 2D** (rectangles, chemins, arcs, textes, images, `measureText` sur la
vraie fonte) · **modules ES** (`<script type="module">`, `import` résolu et
chargé sur le réseau) · **témoins**, **cache HTTP** et **`localStorage`**
persistés sur le disque ·
**images** PNG/JPEG/GIF/BMP décodées par Qt · **vidéo et audio** H.264/AAC via
libavcodec, avec Media Source Extensions et sortie AC'97 · HTTP et HTTPS avec
redirections et jeux de caractères · `file://` · historique avant/arrière,
liens cliquables, défilement.

### Ce qu'il ne sait pas faire

**Transformations en trois dimensions** — `rotateX`, `perspective`, `matrix3d`
sont laissés de côté plutôt qu'aplatis à tort.
**Dégradés coniques** — `linear-gradient` et `radial-gradient` sont peints,
`conic-gradient` non.
**Interpolation exacte des rotations** — une `transition` sur un `transform`
mélange les matrices, ce qui est exact pour les translations et les homothéties
mais passe par l'aplatissement pour un demi-tour.
**Motifs et composition de canvas** — `createPattern` et
`globalCompositeOperation` ne sont pas rendus ; tout le reste du contexte 2D
l'est, pixels compris. **Détourage par forme** — `clip()` suit la boîte du
chemin, ce qui est exact après un `rect()` et approximatif au-delà.
**Chargement parallèle des modules** — le graphe d'`import` est rapporté module
par module, comme l'exige le chargeur synchrone de QuickJS ; un navigateur les
téléchargerait de front.
**Lignes nommées de grille**, **placement dense**, et
**`animation-composition`**. Ce qui manque est listé, avec le reste, dans la
feuille de route (`docs/ROADMAP.md`).

### Ce qui le rend rapide

Sous émulation, avec la pile TCP du noyau, l'essentiel du temps de chargement
n'était ni le calcul ni le transfert : c'était l'attente.

| Mesure | Avant | Après |
|---|---|---|
| Connexions pour une page de 20 images | 20 TCP + 20 TLS | 4, réutilisées |
| Sous-ressources | une à la fois | 4 en parallèle |
| Corps HTML | tel quel | `gzip` (÷5 environ) |
| Mise en page, feuille de 1600 règles | 3,57 s | 0,059 s (**×60**) |

Le facteur soixante vient de l'index : sans lui, styler un élément coûtait un
essai **par règle de la feuille**. Sur une page de 800 éléments et une feuille
de 1600 règles, cela faisait plus d'un million d'essais par mise en page — et la
mise en page est refaite à chaque battement du JavaScript. L'index range les
règles par ce que leur dernier maillon exige (identifiant, classe, balise) et
n'en propose qu'une poignée par élément. Il ne change jamais le résultat : une
règle qu'il écarte est une règle qui n'aurait pas correspondu.

La cascade elle-même n'est reconstruite que si les feuilles ont changé — un
`setTimeout` qui touche au DOM ne réanalyse plus 1600 règles à chaque tour.

### Résolution de noms

`socket.getaddrinfo` ne fonctionne pas dans un binaire glibc statique : la glibc
y délègue à ses modules NSS, qui sont des bibliothèques partagées chargées par
`dlopen`. Le navigateur porte donc son propre client DNS (`moteur/reseau.py`) —
une requête A en UDP vers le serveur de `/etc/resolv.conf`.

La requête part **avant** que le délai d'attente soit posé sur la prise : une
prise déjà passée en non bloquant fait échouer la première émission vers un hôte
dont l'adresse matérielle n'est pas encore connue, et le noyau rend alors
`ENETUNREACH`. C'est un défaut côté noyau, noté dans la feuille de route.

### Le client léger — 100 % du web, rendu ailleurs

Le moteur natif affiche beaucoup, et il n'affichera jamais tout : une
application qui compile son interface, un lecteur qui pousse ses segments, une
page qui dessine en WebGL demandent Blink ou WebKit. Le client léger prend
l'autre chemin, celui d'Opera Mini et de Puffin — **le rendu se fait sur
l'hôte**, avec un vrai Chromium, et l'OS n'affiche que l'image.

```sh
cd tools/render-proxy && npm run setup && npm start   # sur l'hôte
```

Puis, dans le navigateur : **F2** bascule la page courante entre moteur natif
et rendu distant, ou `distant:https://…` dans la barre d'adresse.

| | Moteur natif | Rendu distant |
|---|---|---|
| Couverture | large, jamais totale | **tout le web** |
| Page | un arbre vivant | une image |
| Sélection, recherche | oui | non |
| Dépendance | aucune | un hôte qui tourne |

Ni l'un ni l'autre ne remplace l'autre, d'où la bascule plutôt qu'un choix
définitif.

Le protocole tient en sept requêtes (`/wv/open`, `/wv/shot`, `/wv/click`,
`/wv/scroll`, `/wv/type`, `/wv/key`, `/wv/info`). Les images partent en **JPEG**
— 48 ko contre 300 ko en PNG, ce qui sur le lien d'une machine émulée fait la
différence entre dix images par seconde et une — et une image identique n'est
**pas** retransmise : le client joint la signature de celle qu'il a, le service
répond 304. Une page immobile ne coûte donc rien, et toute la bande passante
reste aux pages qui bougent.

```sh
BO_RENDU=http://127.0.0.1:8080 python3 navigateur/distant-probe.py https://pypi.org
```

### Éprouver YouTube contre le vrai service

```sh
python3 navigateur/youtube-probe.py                     # sur la machine
exec /bo-navigateur /usr/share/bo-navigateur/youtube-probe.py   # sous l'OS
```

Les vérifications de `test_moteur.py` substituent le réseau : elles établissent
que le moteur fait ce qu'il faut d'une réponse donnée, jamais que YouTube rend
cette réponse-là aujourd'hui. Cette sonde fait l'inverse — elle parle au vrai
service et dit **à quelle étape** ça s'arrête : joignabilité, réponse du
lecteur (quel client sert, lequel refuse et pourquoi), choix du flux,
signature, puis lecture réelle d'une tranche.

La dernière étape est celle qui prouve le plus : elle demande la même tranche
avec l'agent du client qui a obtenu l'adresse, puis avec un autre. Google lie
l'adresse au client — si le second rend 403 et le premier 206, c'est exactement
le défaut que la propagation de l'agent corrige.

**Derrière un mandataire à liste blanche**, il faut y autoriser
`www.youtube.com`, `youtubei.googleapis.com`, `*.googlevideo.com`, `s.ytimg.com`
et `i.ytimg.com`. Sans le joker sur `googlevideo.com`, l'extraction réussit et
la lecture échoue : les noms d'hôte du média sont tirés au sort à chaque
requête.

### Ce qui survit à l'extinction

Le RAMFS oublie tout. `/persist` est la zone qui n'oublie pas : le noyau la
déplie du disque au démarrage et l'y réécrit sur `sync`.

```
disque de données (hdb)
├── archive tar          lue au démarrage, depuis le début
└── zone persistante     8 Mio, écrite depuis la fin
```

Les deux ne se rencontrent jamais tant que l'image porte les deux, ce dont
`mkdisk.sh` se charge. La zone est réécrite en entier à chaque `sync` — ni
allocateur de blocs ni table d'inodes, ce qui convient à quelques mégaoctets
écrits rarement. L'en-tête part **en dernier** : jusque-là la zone porte encore
l'ancienne magie, donc l'ancien contenu, et une coupure laisse la version
précédente plutôt qu'un mélange des deux.

`sync` écrit tout ; `fsync` n'écrit que si le descripteur désigne un fichier de
`/persist` — un programme en émet sans compter, et chacun coûterait sinon une
réécriture complète.

Le navigateur y range ses témoins, son cache HTTP et son `localStorage`
(`moteur/stockage.py`). Sans zone persistante — sur la machine de
développement — tout continue en mémoire seule.

`tools/test.sh` démarre **deux fois** sur la même image : le premier passage
écrit, le second doit retrouver. La sonde reconnaît seule son passage, ce qui
évite de refabriquer l'image entre les deux — refabriquer effacerait justement
ce qu'on vérifie.

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

- **`evaluate_js` et `window.pywebview.api`** — le moteur exécute bien le
  JavaScript de la page (QuickJS, §12), mais l'adaptateur pywebview ne branche
  pas encore l'évaluation à la demande depuis Python. Il lève une exception
  explicite plutôt que de rendre `None` en silence.
- **Les applications servant des fichiers locaux** (`create_window(url='index.html')`)
  — pywebview les sert par un serveur HTTP interne, qui a besoin de `listen` et
  `accept`. Le noyau ne les implémente pas encore ; c'est le prochain manque à
  combler pour cette pile.
- **JavaScript : le langage, et une bonne part du navigateur.** QuickJS exécute
  l'ECMAScript en entier, et `moteur/js.py` expose le DOM, les événements, les
  minuteries, `XMLHttpRequest`, `fetch`, le canvas 2D — pixels, dégradés et
  ombres compris —, les Web Components avec un DOM d'ombre réellement isolé, les
  trois observateurs et un `getComputedStyle` résolu ; la liste exacte est au
  §12, et 532 vérifications la tiennent. Ce qui reste hors de portée : WebGL, le
  chiffrement du contenu, et les applications qui compilent leur interface.
- **Vidéo et audio : la chaîne existe, les sites de lecture non.** Le son sort
  par le pilote AC'97 (`/dev/dsp`), libavcodec décode H.264, VP9, AAC et Opus, et
  `<video>`, `<audio>`, `MediaSource` et `SourceBuffer` sont implémentés. Ce qui
  manque à un site de lecture réel est ailleurs : le chiffrement (EME/Widevine)
  pour son catalogue, et le débit qu'une machine émulée sans accélération
  matérielle ne tient pas en 1080p.
- **Plusieurs fenêtres à l'écran en même temps** — le framebuffer n'a pas de
  gestionnaire de fenêtres. Les fenêtres suivantes sont créées et pilotables,
  mais s'affichent l'une après l'autre dans la même surface.
