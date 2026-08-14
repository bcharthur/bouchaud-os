# Matrice de portabilite — le suivi commun des trois chantiers

Ce tableau est le point de rencontre entre la compatibilite Linux et le portage
Ladybird : une meme primitive sert souvent aux deux. Il evite d'implementer deux
fois, ou de croire acquis ce qui ne l'est pas.

**Regle : aucune valeur n'est inventee.** Chaque ligne renvoie a une verification
dans le code au commit `ae7a0d5`. « partiel » signifie que la primitive existe
mais que sa conformite n'a pas ete confrontee a ce qu'attend le consommateur.

| Primitive | Bouchaud | Requis Ladybird | Requis ABI Linux | Verifie dans |
|---|---|---|---|---|
| `mmap` anonyme | oui | oui | oui | `abi/mem.rs` |
| `mmap` fichier `MAP_SHARED` | oui | oui | oui | `abi/mem.rs`, `kernel/partage.rs` |
| `mprotect` | oui | oui | oui | dispatch `mod.rs` |
| `brk` | oui | oui | oui | `abi/mem.rs` |
| `memfd_create` | oui | oui | oui | `abi/file.rs` |
| `shm_open` | via memfd | partiel | oui | — |
| `futex` (wait/wake) | oui | oui | oui | `task.rs` |
| `clone(CLONE_THREAD)` | oui | oui | oui | `abi/proc.rs` |
| `clone3` | **absent** | non | oui | `nr.rs` |
| `set_robust_list` | oui | oui | oui | dispatch |
| Bornes de pile de fil exactes | **inconnu** | **oui (GC)** | oui | a mesurer |
| `fork` / `vfork` | oui | oui | oui | `abi/proc.rs` |
| `execve` | oui | oui | oui | `kernel/exec.rs` |
| `wait4` | oui | oui | oui | dispatch |
| `posix_spawn` | via musl | oui | oui | libc, pas syscall |
| `poll` / `ppoll` | oui | oui | oui | `abi/file.rs` |
| `poll` a grande echelle | **partiel** | oui | oui | balayage lineaire |
| `select` | oui | partiel | oui | dispatch |
| `epoll_create1` | oui | non | oui | dispatch |
| `eventfd2` | oui | oui | oui | `fd.rs` |
| `timerfd` | oui | oui | oui | `fd.rs` |
| `socketpair` | oui | **oui** | oui | `abi/net.rs` |
| `SCM_RIGHTS` | oui | **oui** | oui | `abi/net.rs` |
| `socket`/`connect`/`bind` | oui | oui | oui | `abi/net.rs` |
| `sendmsg` / `recvmsg` | oui | oui | oui | `abi/net.rs` |
| Signaux (`rt_sigaction`…) | oui | oui | oui | `kernel/signal.rs` |
| `clock_gettime` | oui | oui | oui | dispatch |
| `nanosleep` | oui | oui | oui | dispatch |
| `getrandom` | oui | oui | oui | dispatch |
| `prlimit64` (`RLIMIT_AS`) | oui | oui | oui | dispatch |
| `renameat` | **absent** | oui | oui | `nr.rs` |
| `statx` | oui | oui | oui | `abi/file.rs` |
| `readlink` | oui | oui | oui | dispatch |
| `/proc/self/exe` | **absent** | oui | oui | `sysroot.rs` |
| `/proc/self/maps` | **absent** | **oui (GC)** | oui | `sysroot.rs` |
| ELF statique-PIE | oui | oui | oui | `kernel/elf.rs` |
| ELF dynamique + `PT_INTERP` | oui | oui | oui | `kernel/elf.rs` |
| `auxv` complet | partiel | oui | oui | `kernel/elf.rs` |
| vDSO | absent | non | partiel | — |
| TLS (`arch_prctl`) | oui | oui | oui | dispatch |
| Chaine C++23 pour la cible | **inconnu** | **oui** | non | a mesurer |
| Rust pour la cible userland | absent | oui (LibUnicode) | non | — |

## Les six dettes du chemin critique

Classees par ce qu'elles bloquent, pas par difficulte :

1. **Chaine C++23 pour la cible** — bloque tout le chantier Ladybird. A mesurer
   avant toute autre chose (`LIBJS_PORT.md` etape 0).
2. **Bornes de pile de fil** — bloque un GC correct. Se manifeste tard et mal.
3. **`/proc/self/maps`** — meme sujet, autre chemin.
4. **`poll` a grande echelle** — la boucle Ladybird surveille bien plus de
   descripteurs que notre navigateur.
5. **`clone3`** — cout d'un appel inutile par thread aujourd'hui, correction
   simple.
6. **`renameat`** — ecriture atomique.

Aucune n'est un obstacle de conception. La premiere est la seule qui puisse
remettre en cause le calendrier.
