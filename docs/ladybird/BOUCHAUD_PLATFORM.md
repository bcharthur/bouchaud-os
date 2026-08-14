# La couche plateforme Bouchaud pour Ladybird

## Principe

Ladybird ne doit pas apprendre l'existence de Bouchaud OS par des `#ifdef`
disperses. Le depot montre qu'upstream traite deja la portabilite par
**substitution de fichier** : `SocketWindows.cpp` face au `Socket.cpp` Unix,
`EventLoopImplementationWindows.cpp` face a `EventLoopImplementationUnix.cpp`.

Bouchaud s'insere par la meme couture, et nulle part ailleurs.

## Ou Bouchaud se branche

Deux regimes, et il faut choisir consciemment :

**Regime A — Bouchaud se fait passer pour Unix.** Le userland est deja musl ;
l'ABI expose 153 appels Linux. La plupart de LibCore compile alors *sans
modification*. C'est le regime par defaut et il faut le pousser aussi loin que
possible : chaque fichier qui compile tel quel est un fichier qu'on n'aura pas a
maintenir.

**Regime B — variante Bouchaud explicite.** Reserve a ce qui n'a pas
d'equivalent Unix chez nous : la surface graphique (protocole GUI v1), le bac a
sable, les statistiques de processus. La convention suit celle d'upstream :

    Libraries/LibCore/XxxBouchaud.cpp
    Services/RendererSandboxBouchaud.cpp

Le critere de bascule de A vers B est mesurable : on ne cree un fichier Bouchaud
que lorsque le fichier Unix **echoue a compiler ou a s'executer**, jamais par
precaution.

## Matrice LibCore -> Bouchaud

Etat verifie dans `src/kernel/`. « partiel » signifie que la primitive existe
mais que son comportement n'a pas ete confronte a ce qu'attend Ladybird.

| Interface LibCore | Primitive Bouchaud | Etat |
|---|---|---|
| `EventLoop`, `EventLoopImplementationUnix` | `poll`/`ppoll`/`select` + `timerfd` + `eventfd` | supporte |
| `Notifier` | `poll` sur descripteur | supporte |
| `Timer`, `ElapsedTimer` | `clock_gettime`, `timerfd_create` | supporte |
| `Process` (spawn) | `fork`/`vfork`/`execve`/`wait4` ; `posix_spawn` via musl | supporte |
| `File`, `Directory` | `openat`, `getdents64`, `statx`, RAMFS | supporte |
| `MappedFile` | `mmap` fichier + `MAP_SHARED` (cache de pages) | supporte |
| `Socket`, `LocalServer`, `TCPServer` | pile reseau + `socket`/`bind`/`listen`/`accept` | partiel — a eprouver sous charge |
| `Socketpair` | `socketpair` (deux canaux croises) | supporte |
| `AnonymousBuffer` | `memfd_create` + `mmap(MAP_SHARED)` | supporte |
| `System` (enveloppes) | l'essentiel des 153 appels | supporte |
| `StandardPaths` | `$HOME`, `/tmp`, `/etc` presents | partiel — `XDG_*` a definir |
| `Environment` | `envp` construit par `exec` | supporte |
| `ThreadEventQueue` / LibThreading | `clone(CLONE_THREAD)` + `futex` via pthread musl | partiel — a eprouver a N fils |
| `TimeZoneWatcher` | aucun equivalent | absent — variante Bouchaud inerte |
| `Platform/ProcessStatistics` | `/proc` partiel | partiel |
| `Random` | `getrandom`, `/dev/urandom` | supporte |

## Ce qui manque cote noyau

Etabli en croisant la surface POSIX mesuree de Ladybird avec le dispatch reel :

1. **`clone3`** — ENOSYS aujourd'hui. musl recent l'essaie puis retombe sur
   `clone` ; non bloquant, mais a implementer avant de multiplier les fils.
2. **`renameat`** — absent. LibCore/LibFileSystem s'en sert pour l'ecriture
   atomique (fichier temporaire puis renommage).
3. **Semantique de `poll` a grande echelle** — la boucle d'evenements de Ladybird
   surveille beaucoup plus de descripteurs que notre navigateur actuel. Notre
   `sys_poll` est un balayage lineaire avec attente active bornee ; correct, mais
   son cout croit avec le nombre de descripteurs.
4. **Bornes de pile des fils** — le GC de LibJS balaie les piles. `clone` doit
   exposer des bornes exactes, sinon le GC recolte des objets vivants.

Ces quatre points sont les seules dettes noyau connues pour M1–M4. Elles sont
reportees dans `../PORTABILITY_MATRIX.md`.

## Ce qu'on ne fait pas

- Pas de reimplementation de LibCore. Si une fonction manque, on ajoute la
  primitive systeme, pas une variante de la fonction.
- Pas de fork divergent de la boucle d'evenements : `EventLoopImplementationUnix`
  doit fonctionner tel quel. Si elle n'y arrive pas, c'est notre `poll` qui est
  en cause.
