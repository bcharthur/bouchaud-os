# Userland de Bouchaud OS

Le noyau expose désormais l'ABI Linux x86-64 : un binaire compilé pour Linux
s'exécute en ring 3 sans recompilation du noyau. Ce dossier contient la chaîne
de construction côté utilisateur.

```
./build.sh freestanding    # tests d'ABI sans libc (gcc + ld, aucune dépendance)
./build.sh musl            # binaires statiques musl
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

Le noyau expose une surface de type Linux, celle qu'attend le plugin `linuxfb`
de Qt :

| Chemin | Rôle |
|---|---|
| `/dev/fb0` | framebuffer 1280x720 XRGB8888, `mmap`-able sur la VRAM réelle |
| `/dev/input/event0` | clavier, format `struct input_event` evdev |
| `/dev/input/event1` | souris (position absolue + bouton) |

Ioctls implémentés : `FBIOGET_VSCREENINFO`, `FBIOGET_FSCREENINFO`,
`FBIOPUT_VSCREENINFO`, `FBIOPAN_DISPLAY`, `EVIOCGVERSION`, `EVIOCGNAME`,
`EVIOCGID`, plus `TCGETS`/`TIOCGWINSZ` sur la console.

Ouvrir `/dev/fb0` bascule automatiquement la carte en mode graphique ; le mode
texte est rendu au shell quand le programme se termine.

`fb-demo.c` valide toute cette chaîne sans aucune bibliothèque : il ouvre le
framebuffer, lit sa géométrie par ioctl, le mappe et dessine dedans.

Variables d'environnement fournies par défaut à tout programme (voir
`kernel::exec::default_environment`) :

```
QT_QPA_PLATFORM=linuxfb
QT_QPA_EVDEV_KEYBOARD_PARAMETERS=/dev/input/event0
QT_QPA_EVDEV_MOUSE_PARAMETERS=/dev/input/event1
SDL_VIDEODRIVER=fbcon  SDL_FBDEV=/dev/fb0
```

Pour Qt lui-même, une compilation croisée statique est nécessaire :

```sh
./configure -static -release -no-opengl -no-xcb -no-feature-vulkan \
            -qpa linuxfb -platform linux-g++ \
            -device-option CROSS_COMPILE=musl- -prefix /usr
```

Ce qui manque encore côté noyau pour une pile Qt complète, par ordre
d'importance :

1. **`execve`** — non implémenté (le shell lance les programmes directement) ;
2. **`fork`** — non implémenté : demande la copie paresseuse (COW) de l'espace
   d'adressage. `clone(CLONE_THREAD)` suffit à `pthread`, donc à Qt et Python,
   mais pas à un shell POSIX ;
3. **signaux** — `rt_sigaction` accepte et ignore ; aucun gestionnaire n'est
   appelé. Suffisant tant que le programme n'attend pas `SIGCHLD`/`SIGALRM` ;
4. **sockets** — la pile TCP/IP du noyau existe (`src/net/`) mais n'est pas
   reliée à l'ABI POSIX ; il n'y a donc pas encore de `socket()`/`connect()` ;
5. **cache de pages partagé** pour `mmap` de fichier (voir §7).

## Tests d'ABI sans libc

`ring3-selftest.c` vérifie, sans dépendance : `uname`, `getpid`/`gettid`, `brk`,
`mmap`/`mprotect`, TLS par `arch_prctl`, `clock_gettime`, `clone` d'un thread,
`futex` (réveil croisé), `openat`/`read`/`write`/`lseek`, `writev`,
`exit_group`.

Version embarquée dans le noyau, sans aucun fichier : la commande `usermode`
génère un ELF64 en mémoire et l'exécute. C'est le moyen le plus rapide de
vérifier que le ring 3 est fonctionnel après une modification du noyau.
