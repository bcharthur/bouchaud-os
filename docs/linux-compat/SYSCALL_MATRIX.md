# Matrice des appels systeme Linux

*Genere par lecture de `src/kernel/abi/nr.rs` (declarations) et
`src/kernel/abi/mod.rs` (dispatch reel) au commit `ae7a0d5`. Aucune valeur n'est
saisie a la main.*

## Resume

| Mesure | Valeur |
|---|---|
| Appels declares dans `nr.rs` | 162 |
| Appels **routes** dans `mod.rs` | **153** |
| Appels analyses ci-dessous | 103 |
| Dont routes | **101** |
| Manquants | `clone3`, `renameat` |

La colonne « Bouchaud » vaut `oui` quand le numero est declare **et** route vers
une implementation. « declare, non route » signale un numero connu sans
traitement — un piege : l'appel ne rend pas `ENOSYS` mais tombe dans le cas par
defaut.

Les colonnes « Requis » viennent de l'analyse de `docs/ladybird/DEPENDENCIES.md`
(surface POSIX mesuree par bibliotheque), pas d'une intuition.

### Fichiers / descripteurs

| Syscall | n° Linux | Bouchaud | Requis LibJS | Requis LibCore/LibIPC | Requis apps Linux |
|---|---|---|---|---|---|
| `read` | 0 | oui | X | X | X |
| `write` | 1 | oui | X | X | X |
| `pread64` | 17 | oui |  |  | X |
| `pwrite64` | 18 | oui |  |  | X |
| `openat` | 257 | oui | X | X | X |
| `close` | 3 | oui | X | X | X |
| `lseek` | 8 | oui | X | X | X |
| `fstat` | 5 | oui | X | X | X |
| `stat` | 4 | oui |  |  | X |
| `newfstatat` | 262 | oui |  |  | X |
| `statx` | 332 | oui |  |  | X |
| `getdents64` | 217 | oui |  |  | X |
| `readlink` | 89 | oui |  |  | X |
| `fcntl` | 72 | oui |  | X | X |
| `dup` | 32 | oui |  |  | X |
| `dup2` | 33 | oui |  | X | X |
| `dup3` | 292 | oui |  | X | X |
| `pipe2` | 293 | oui |  | X | X |
| `ioctl` | 16 | oui |  | X | X |
| `getcwd` | 79 | oui |  |  | X |
| `chdir` | 80 | oui |  |  | X |
| `ftruncate` | 77 | oui |  |  | X |
| `unlinkat` | 263 | oui |  |  | X |
| `mkdirat` | 258 | oui |  |  | X |
| `renameat` | — | **absent** |  |  | X |
| `fsync` | 74 | oui |  |  | X |
| `access` | 21 | oui |  |  | X |
| `faccessat` | 269 | oui |  |  | X |

### Memoire

| Syscall | n° Linux | Bouchaud | Requis LibJS | Requis LibCore/LibIPC | Requis apps Linux |
|---|---|---|---|---|---|
| `mmap` | 9 | oui | X | X | X |
| `munmap` | 11 | oui | X | X | X |
| `mprotect` | 10 | oui | X | X | X |
| `mremap` | 25 | oui |  |  | X |
| `madvise` | 28 | oui | X | X | X |
| `msync` | 26 | oui |  |  | X |
| `brk` | 12 | oui | X | X | X |
| `memfd_create` | 319 | oui |  | X | X |

### Processus / threads

| Syscall | n° Linux | Bouchaud | Requis LibJS | Requis LibCore/LibIPC | Requis apps Linux |
|---|---|---|---|---|---|
| `clone` | 56 | oui | X | X | X |
| `clone3` | — | **absent** |  |  | X |
| `fork` | 57 | oui |  | X | X |
| `vfork` | 58 | oui |  |  | X |
| `execve` | 59 | oui |  | X | X |
| `exit` | 60 | oui |  |  | X |
| `exit_group` | 231 | oui | X | X | X |
| `wait4` | 61 | oui |  | X | X |
| `getpid` | 39 | oui |  | X | X |
| `getppid` | 110 | oui |  |  | X |
| `gettid` | 186 | oui |  |  | X |
| `set_tid_address` | 218 | oui | X | X | X |
| `set_robust_list` | 273 | oui | X | X | X |
| `arch_prctl` | 158 | oui | X | X | X |
| `prctl` | 157 | oui |  |  | X |
| `sched_yield` | 24 | oui |  |  | X |
| `prlimit64` | 302 | oui | X | X | X |
| `getrlimit` | 97 | oui |  |  | X |
| `setrlimit` | 160 | oui |  |  | X |

### Synchronisation

| Syscall | n° Linux | Bouchaud | Requis LibJS | Requis LibCore/LibIPC | Requis apps Linux |
|---|---|---|---|---|---|
| `futex` | 202 | oui | X | X | X |
| `nanosleep` | 35 | oui | X | X | X |
| `clock_nanosleep` | 230 | oui |  |  | X |

### Signaux

| Syscall | n° Linux | Bouchaud | Requis LibJS | Requis LibCore/LibIPC | Requis apps Linux |
|---|---|---|---|---|---|
| `rt_sigaction` | 13 | oui | X | X | X |
| `rt_sigprocmask` | 14 | oui | X | X | X |
| `rt_sigreturn` | 15 | oui |  |  | X |
| `rt_sigsuspend` | 130 | oui |  |  | X |
| `kill` | 62 | oui |  | X | X |
| `tgkill` | 234 | oui |  |  | X |
| `sigaltstack` | 131 | oui |  |  | X |

### Attente d'evenements

| Syscall | n° Linux | Bouchaud | Requis LibJS | Requis LibCore/LibIPC | Requis apps Linux |
|---|---|---|---|---|---|
| `poll` | 7 | oui |  | X | X |
| `ppoll` | 271 | oui |  | X | X |
| `select` | 23 | oui |  |  | X |
| `pselect6` | 270 | oui |  |  | X |
| `epoll_create1` | 291 | oui |  | X | X |
| `epoll_ctl` | 233 | oui |  |  | X |
| `epoll_wait` | 232 | oui |  |  | X |
| `eventfd2` | 290 | oui |  | X | X |
| `timerfd_create` | 283 | oui |  | X | X |
| `timerfd_settime` | 286 | oui |  |  | X |

### Reseau

| Syscall | n° Linux | Bouchaud | Requis LibJS | Requis LibCore/LibIPC | Requis apps Linux |
|---|---|---|---|---|---|
| `socket` | 41 | oui |  | X | X |
| `socketpair` | 53 | oui |  | X | X |
| `connect` | 42 | oui |  | X | X |
| `bind` | 49 | oui |  |  | X |
| `listen` | 50 | oui |  |  | X |
| `accept` | 43 | oui |  |  | X |
| `accept4` | 288 | oui |  |  | X |
| `sendto` | 44 | oui |  |  | X |
| `recvfrom` | 45 | oui |  |  | X |
| `sendmsg` | 46 | oui |  | X | X |
| `recvmsg` | 47 | oui |  | X | X |
| `getsockopt` | 55 | oui |  |  | X |
| `setsockopt` | 54 | oui |  |  | X |
| `shutdown` | 48 | oui |  |  | X |
| `getsockname` | 51 | oui |  |  | X |
| `getpeername` | 52 | oui |  |  | X |

### Temps / divers

| Syscall | n° Linux | Bouchaud | Requis LibJS | Requis LibCore/LibIPC | Requis apps Linux |
|---|---|---|---|---|---|
| `clock_gettime` | 228 | oui | X | X | X |
| `clock_getres` | 229 | oui |  |  | X |
| `gettimeofday` | 96 | oui |  |  | X |
| `time` | 201 | oui |  |  | X |
| `getrandom` | 318 | oui | X | X | X |
| `uname` | 63 | oui |  |  | X |
| `sysinfo` | 99 | oui |  |  | X |
| `getuid` | 102 | oui |  |  | X |
| `geteuid` | 107 | oui |  |  | X |
| `getgid` | 104 | oui |  |  | X |
| `getegid` | 108 | oui |  |  | X |
| `umask` | 95 | oui |  |  | X |

## Les deux manques

**`clone3`** — musl recent tente `clone3` puis retombe sur `clone` en cas
d'`ENOSYS`. Non bloquant aujourd'hui (observe dans les journaux du navigateur),
mais a implementer avant de multiplier les fils : le repli coute un appel inutile
a chaque creation de thread.

**`renameat`** — utilise par LibCore et LibFileSystem pour l'ecriture atomique
(ecrire un fichier temporaire puis renommer). Son absence ne casse pas la
lecture, mais rend toute ecriture de configuration non atomique.

## Ce que cette table ne dit pas

Elle mesure la **presence**, pas la **conformite**. Un appel route peut differer
de Linux sur un cas limite, et c'est le defaut le plus couteux a diagnostiquer
puisqu'il ne produit aucune erreur. La reponse est le traceur decrit dans
`MASTER_PLAN.md` : confronter le comportement a des traces reelles, application
par application.
