# Userland de Bouchaud OS

Le noyau expose désormais l'ABI Linux x86-64 : un binaire compilé pour Linux
s'exécute en ring 3 sans recompilation du noyau. Ce dossier contient la chaîne
de construction côté utilisateur.

```
./build.sh freestanding    # tests d'ABI sans libc (gcc + ld, aucune dépendance)
./build.sh musl            # binaires statiques musl (dont qpa-probe)
./build.sh musl-dynamic    # binaires dynamiques + ld-musl-x86_64.so.1
```

Les binaires produits vont dans `out/`. Il faut ensuite les placer dans le
RAMFS de l'OS, puis :

```
exec /hello
elfinfo /hello        # type ELF, segments, interpréteur requis
tasks                 # threads du programme en cours
strace on             # trace des appels système sur COM1
```

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
| 4. Appels POSIX | ~75 appels, numéros et structures Linux | `src/kernel/abi/` |
| 5. Processus / threads | `clone(CLONE_THREAD)`, futex, préemption | `src/kernel/task.rs` |
| 6. libc musl | **côté utilisateur** — voir ci-dessous | ce dossier |
| 7. `ld.so` | chargé par le noyau, résout en ring 3 | `src/kernel/exec.rs` |
| 8. Runtime C++ | **côté utilisateur** — voir ci-dessous | ce dossier |
| 9. Serveur graphique | `/dev/fb0` mmap + ioctls fbdev + evdev | `src/kernel/{fd,abi/file}.rs` |

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

Limite actuelle : `mmap` de fichier est une copie privée, pas un cache de pages
partagé. Deux processus chargeant la même bibliothèque ne partagent donc pas
ses pages — correct fonctionnellement (`MAP_PRIVATE`), plus coûteux en mémoire.

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
bibliothèque.

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
./configure -static -release -no-opengl -no-xcb -no-feature-vulkan \
            -no-feature-dbus -no-feature-glib \
            -qpa linuxfb -platform linux-g++ \
            -device-option CROSS_COMPILE=musl- -prefix /usr
```

`-no-feature-glib` importe : sans lui, Qt utilise le dispatcher GLib, qui
demande davantage au système que sa propre boucle `poll`.

### Ce qui manque encore, par ordre d'importance

1. **`fork`** et **`execve`** — non implémentés. `fork` demande la copie
   paresseuse (COW) de l'espace d'adressage. `clone(CLONE_THREAD)` suffit à
   `pthread`, donc à Qt et à Python, mais pas à `QProcess` ni à un shell POSIX ;
2. **signaux** — `rt_sigaction` accepte et ignore ; aucun gestionnaire ring 3
   n'est appelé. Seuls les signaux fatals (`SIGABRT`, `SIGKILL`, `SIGSEGV`,
   `SIGTERM`) agissent, en terminant le processus. Suffisant tant que le
   programme n'attend pas `SIGCHLD` ou `SIGALRM` ;
3. **sockets** — la pile TCP/IP du noyau existe (`src/net/`) mais n'est pas
   reliée à l'ABI POSIX : pas encore de `socket()`/`connect()`, donc pas de
   `QtNetwork` ni de `QLocalSocket` ;
4. **cache de pages partagé** pour `mmap` de fichier (voir §7) ;
5. **accélération** — tout le rendu est logiciel, et `present()` recopie le
   tampon. Une pile Qt tournera, mais au rythme d'un rendu logiciel en
   1280x720.

## Tests d'ABI sans libc

`ring3-selftest.c` vérifie, sans dépendance : `uname`, `getpid`/`gettid`, `brk`,
`mmap`/`mprotect`, TLS par `arch_prctl`, `clock_gettime`, `clone` d'un thread,
`futex` (réveil croisé), `openat`/`read`/`write`/`lseek`, `writev`,
`exit_group`.

Version embarquée dans le noyau, sans aucun fichier : la commande `usermode`
génère un ELF64 en mémoire et l'exécute. C'est le moyen le plus rapide de
vérifier que le ring 3 est fonctionnel après une modification du noyau.
