# Porter un vrai moteur web dans Bouchaud OS

Objectif : **100 % du web, sans hôte**. Aujourd'hui l'OS a deux façons d'afficher
une page, et aucune ne tient cette promesse :

| | Couverture | Autonome |
|---|---|---|
| Moteur natif (`tools/userland/navigateur/moteur/`) | large, jamais totale | oui |
| Client léger (`moteur/distant.py` + Chromium sur l'hôte) | **totale** | non |

Seuls trois moteurs au monde rendent la totalité du web : Blink, WebKit, Gecko.
Il n'existe aucune bibliothèque Python ou Qt qui l'évite — `PyQtWebEngine`,
`cefpython`, `QtWebEngine` sont toutes des liaisons vers ces mêmes moteurs. Le
raccourci n'existe pas ; il faut porter.

## Le candidat

**WPE WebKit.** Conçu pour l'embarqué : pas de serveur graphique, il rend dans
un tampon via un *backend* qu'on écrit soi-même. ~2,5 M lignes, contre ~7 M pour
Chromium, et sans l'architecture de bac à sable de ce dernier.

Servo (Rust, `libservo`) reste une option intéressante — bien plus petit, et il
parlerait la langue du noyau — mais son API d'embarquement bouge encore.

QtWebEngine est écarté : c'est Chromium, il faut ~100 Go et des heures pour le
compiler, et il réclame des primitives que l'OS est loin d'avoir.

## Ce que l'OS a déjà, et qui est le vrai prérequis

Ce n'est pas rien, et c'est ce qui rend l'entreprise envisageable :

- ABI Linux x86-64 en ring 3, **111 appels système**
- ELF64 + `ld.so`, donc les bibliothèques partagées
- `fork`/`execve`/`wait4`, `clone`, futex, signaux
- `mmap` avec `MAP_SHARED` sur fichier, adossé à un cache de pages
- sockets POSIX sur une pile TCP/IP maison
- framebuffer et evdev
- **mémoire partagée anonyme** (`memfd_create`) et `/dev/shm` — voir plus bas

## Le relevé : ce qui manque, mesuré

Établi en confrontant la table d'appels (`src/kernel/abi/nr.rs`) à ce
qu'exigent WebKit et sa chaîne de dépendances.

### Bloquant pour l'architecture multi-processus

| Manque | Pourquoi c'est bloquant |
|---|---|
| ~~**`SCM_RIGHTS`**~~ | **Fait** — voir phase 2. |
| `listen` / `accept` / `accept4` sur `AF_UNIX` | Le canal de contrôle entre processus. `socketpair` couvre le cas hérité par `fork` ; pas celui d'un processus lancé séparément. |

### Bloquant pour la chaîne de dépendances (GLib, GIO, ICU)

`statfs` / `fstatfs`, `utimensat`, `symlink` / `link`, `inotify_init1` (la
surveillance de fichiers peut se dégrader proprement), `clone3` (glibc récent
l'essaie avant `clone`), `sendfile`, `flock`.

### Non bloquant, à surveiller

`rseq` (glibc l'enregistre au démarrage ; `ENOSYS` suffit), `membarrier`
(présent), `process_vm_readv`, `io_uring`.

### Au-delà du noyau

- **Un système de fichiers inscriptible généraliste.** `/persist` est une zone
  de taille fixe réécrite en entier ; WebKit veut un cache, des bases IndexedDB,
  des profils. Il faut un vrai FS à blocs.
- **Rendu.** WPE veut un backend ; le nôtre écrirait dans `/dev/fb0`. Sans GPU,
  la composition est logicielle — jouable pour une page, coûteux pour une vidéo.
- **Média.** WPE passe par GStreamer. `bomedia` (libavcodec) existe déjà et
  pourrait servir de base à un backend, mais ce n'est pas un raccourci.
- **Polices.** fontconfig + HarfBuzz. Qt les embarque déjà, donc le terrain est
  connu.

## Phase 1 — faite

La mémoire partagée anonyme, substrat de tout moteur multi-processus.

- `memfd_create` (319). Le descripteur est adossé à un **nœud RAMFS anonyme** :
  sans parent, sans nom dans l'arborescence. Le cache de pages étant indexé par
  nœud, deux `mmap(MAP_SHARED)` du même descripteur voient mécaniquement les
  mêmes frames physiques — le partage n'est pas ajouté, il découle.
- `/dev/shm`, où `shm_open` dépose ses segments nommés.

Éprouvé par `tools/userland/shm-probe.c`, joué à chaque `./tools/test.sh` :
16 vérifications, dont celle qui compte — le père écrit, `fork`, le fils relit
puis écrit, le père voit l'écriture du fils, sur la première page **et** sur la
seconde. Si cette dernière échouait, chaque processus aurait sa copie et le
partage serait un mensonge.

## Phase 2 — faite

**Le passage de descripteurs entre processus.** Partager de la mémoire ne sert à
rien si l'on ne peut pas confier ce qui la désigne : `SCM_RIGHTS` est la seule
façon pour deux processus de s'échanger autre chose que des octets.

Un socket local porte désormais deux files : les octets, et les descripteurs.
Ceux-ci sont **copiés** dans le canal — l'émetteur garde le sien, le récepteur en
obtient un à lui — ce qui évite qu'un envoi ferme un fichier sous les pieds de
celui qui l'envoie.

Trois défauts trouvés en chemin, tous invisibles jusqu'à ce qu'on les exerce :

- **la lecture d'une paire de sockets ne bloquait pas.** Elle rendait `EAGAIN`
  immédiatement, ce qui faisait échouer le premier `recvmsg` d'un dialogue —
  celui qui arrive avant que le pair ait eu la main ;
- **`sendmsg` passait par `sendto`,** qui ne connaît ni les paires de sockets ni
  leur absence d'adresse. Les descripteurs partaient, l'octet qui les accompagne
  échouait, et l'émetteur en concluait que rien n'était parti ;
- un `recvmsg` qui ne reçoit **que** des descripteurs — le cas courant — rendait
  une erreur faute d'octets, alors qu'il venait d'obtenir ce qu'on attendait.

Éprouvé par `shm-probe.c` : 21 vérifications, dont celle qui compte — le fils
crée un tampon, y écrit, passe **le descripteur** par un socket ; le père, qui
n'a jamais ouvert ce tampon, le projette et y lit ce que le fils a écrit.

## Phases suivantes

2. **`AF_UNIX` complet** — `listen`, `accept`, sockets nommés, pour les
   processus lancés séparément plutôt qu'hérités par `fork`.
3. **Système de fichiers inscriptible** à blocs, sur le pilote ATA qui sait déjà
   écrire.
4. **Compléter la couche POSIX** — `statfs`, `utimensat`, `symlink`, `clone3`.
5. **Construire la chaîne de dépendances** : ICU, GLib, libsoup, Cairo,
   HarfBuzz, en statique, avec la chaîne déjà employée pour Qt.
6. **Backend WPE** vers `/dev/fb0`, puis premier rendu.

## L'honnêteté sur l'échelle

C'est un travail de plusieurs mois, pas de plusieurs jours, et il peut échouer
sur un obstacle qu'on ne verra qu'en le rencontrant. La méthode retenue est donc
empirique : à chaque phase, une sonde qui prouve la primitive, jouée par
`./tools/test.sh`. On saura à tout moment où l'on en est — et le client léger
donne, pendant ce temps, la totalité du web.
