# Graphe de dependances Ladybird — mesure, pas supposition

Upstream analyse : `cdfe5f858eb5fc64a8d9d3fcc247d71b03fbd1f6`.

Toutes les tables ci-dessous sont **calculees** sur l'arbre reel (comptage des
`#include` par bibliotheque, lecture des `target_link_libraries`). Aucune valeur
n'est estimee.

## 1. Dependances entre bibliotheques

Nombre d'inclusions depuis chaque bibliotheque vers les autres :

| Bibliotheque | Fichiers | Depend de (occurrences) |
|---|---|---|
| `AK` | 196 | **rien** |
| `LibSync` | 14 | AK(50) |
| `LibMain` | 2 | AK(7) |
| `LibRegex` | 4 | AK(14) |
| `LibUnicode` | 47 | AK(152) |
| `LibThreading` | 5 | AK(11), LibSync(2), LibCore(1) |
| `LibCore` | 114 | AK(265), LibSync(9), LibURL(3), LibMain(1), LibUnicode(1) |
| `LibGC` | 56 | AK(116), LibCore(10), LibSync(2), LibThreading(1) |
| `LibIPC` | 39 | AK(106), LibCore(36), LibSync(10), LibURL(7), LibThreading(6) |
| `LibJS` | 556 | AK(630), LibGC(84), LibUnicode(54), LibCrypto(22), LibCore(11), LibTextCodec(3), LibGfx(1), LibSyntax(1), LibWasm(1), LibFileSystem(1), LibRegex(1) |

Lecture : **AK ne depend de rien**. C'est la premiere brique portable, et donc la
premiere PR. `LibUnicode` ne depend d'aucune autre bibliotheque Ladybird — mais
voir la section 3.

## 2. Cloture de liaison reelle (CMake)

    LibGC  PRIVATE LibCore
           PUBLIC  LibSync LibThreading

    LibJS  PRIVATE LibCore LibCrypto LibFileSystem LibRegex LibSyntax
                   LibTextCodec LibGC simdjson::simdjson
           PUBLIC  LibUnicode

    LibUnicode PRIVATE ICU::i18n ICU::uc ICU::data
               PRIVATE libunicode_rust

    LibCrypto  PRIVATE PkgConfig::libtommath
               PUBLIC  OpenSSL::Crypto

Le commentaire d'upstream sur la liaison publique de LibUnicode est explicite :
« Link LibUnicode publicly to ensure ICU data (which is in libicudata.a) is
available in any process using LibJS. »

**Il n'existe donc pas de LibJS sans ICU.**

## 3. Le vrai cout d'entree : le tiers-parti

`vcpkg.json` epingle les versions. Pour la cible minimale « LibJS execute
`1+2` », le sous-ensemble strictement necessaire est :

| Paquet | Version epinglee | Pourquoi | Difficulte sur Bouchaud |
|---|---|---|---|
| `icu` | 78.3 | LibUnicode (Intl, collation, dates) | **elevee** — volumineux, donnees a embarquer |
| `openssl` | 3.6.3 | LibCrypto (public) | moyenne — deja un TLS maison, mais c'est OpenSSL qui est lie |
| `libtommath` | 1.3.0 | BigInt | faible |
| `simdjson` | 4.6.4 | `JSON.parse` | faible (en-tetes + SIMD x86-64) |
| `fast-float` | 8.2.10 | conversion numerique | faible |
| `fmt` | 12.2.0 | formatage AK | faible |
| `mimalloc` | 2.2.7 | allocateur | moyenne — peut etre remplace par celui de musl |
| Rust (`libunicode_rust`) | via `Cargo.toml` | LibUnicode | moyenne — cible Rust supplementaire |

Le reste de `vcpkg.json` (Skia, ANGLE, Vulkan, FFmpeg, HarfBuzz, curl, Qt, SDL3,
sqlite3, woff2, libjxl, …) n'est **pas** requis pour M4. Il le devient a partir de
LibGfx/LibWeb, et fera l'objet d'un document dedie a ce moment-la.

## 4. Surface POSIX mesuree

Occurrences de primitives systeme, fichiers Windows exclus :

| | mmap | mprotect | pthread | fork | posix_spawn | socket | socketpair | poll | memfd | shm_open | signal | clock_gettime | getrandom |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| AK | 4 | · | 27 | 1 | · | · | · | · | · | · | · | 1 | 1 |
| LibSync | · | · | 56 | · | · | · | · | · | · | · | 2 | · | · |
| LibThreading | · | · | 24 | · | · | · | · | · | · | · | 1 | · | · |
| LibCore | 32 | 2 | 15 | 2 | 30 | 11 | 3 | 5 | 2 | 3 | 17 | · | · |
| LibGC | 1 | · | · | · | · | · | · | · | · | · | 1 | · | · |
| LibJS | 2 | · | · | · | · | · | · | · | · | · | · | · | · |
| LibUnicode | · | · | · | · | · | · | · | · | · | · | · | · | · |
| LibIPC | · | · | · | · | · | 1 | 1 | 1 | · | · | 1 | · | · |

Trois enseignements :

1. **LibJS et LibGC ne touchent presque pas le systeme** (3 `mmap` a eux deux).
   Le systeme est atteint via AK, LibSync et LibCore.
2. **`pthread` est la primitive dominante** (122 occurrences). Elle est fournie
   par musl au-dessus de `clone`/`futex`/`set_robust_list`, tous trois presents
   dans Bouchaud.
3. `posix_spawn` (30 occurrences dans LibCore) est une fonction de libc, pas un
   appel systeme : musl l'implemente sur `clone`+`execve`. Ce n'est pas un manque.

## 5. Fichiers dependants de la plateforme

Les paires existantes montrent ou s'inserer :

| Composant | Variante Unix | Variante Windows |
|---|---|---|
| Boucle d'evenements | `EventLoopImplementationUnix.cpp/.h` | `EventLoopImplementationWindows.cpp/.h` |
| Socket | (defaut) | `SocketWindows.cpp` |
| Socketpair | (defaut) | `SocketpairWindows.cpp` |
| Fichier | (defaut) | `FileWindows.cpp` |
| Fichier mappe | (defaut) | `MappedFileWindows.cpp` |
| Processus | (defaut) | `ProcessWindows.cpp` |
| System | (defaut) | `SystemWindows.cpp` |
| Tampon anonyme | (defaut) | `AnonymousBufferWindows.cpp` |
| Serveurs local/TCP/UDP | (defaut) | `*ServerWindows.cpp` |
| Fuseau horaire | `TimeZoneWatcherUnix.cpp` | `TimeZoneWatcherWindows.cpp` |
| Mutex / CondVar / RWLock | (defaut, LibSync) | `*Windows.cpp` |
| Statistiques process | `Platform/ProcessStatisticsLinux.cpp` | — |
| Bac a sable renderer | `Services/RendererSandboxLinux.cpp` | `RendererSandboxUnimplemented.cpp` |

`RendererSandboxUnimplemented.cpp` est notre porte d'entree pour le sandbox : il
existe deja un cas « plateforme sans implementation » qui compile.

## 6. Exigences de compilation

| Exigence | Valeur | Consequence pour Bouchaud |
|---|---|---|
| Standard | **C++23** (`CMAKE_CXX_STANDARD 23`, `REQUIRED ON`) | impose un GCC ≥ 13 / Clang ≥ 16 cible musl |
| Exceptions | `-fno-exceptions` | aligne sur ce que le userland Bouchaud sait deja faire |
| RTTI | non desactive globalement | a verifier a l'edition de liens |
| Rust | `Cargo.toml` + `rust-toolchain.toml` a la racine, `RustAllocator.rs` | une seconde chaine a cross-compiler |

Le `-fno-exceptions` d'upstream est une bonne nouvelle : `tools/userland/README.md`
documente deja `musl-g++ -static-pie -fno-exceptions -fno-rtti` comme chemin
minimal.

## 7. Mecanisme d'integration — decision

Quatre options ont ete pesees contre le seul critere qui compte : **rester proche
de l'upstream sans figer un copier-coller**.

| Option | Suivi upstream | Poids du depot | Verdict |
|---|---|---|---|
| `git subtree` | correct | +27 877 fichiers | rejete : gonfle chaque clone Bouchaud |
| Miroir de branche | bon | eleve | rejete : duplique la maintenance |
| `git submodule` | excellent | nul | **ecarte a l'implementation, voir ci-dessous** |
| SHA epingle + script | excellent | nul | **retenu** |

**Decision revisee.** La premiere redaction de ce document retenait le submodule.
En l'implementant, un defaut est apparu : le SHA aurait ete inscrit a **deux**
endroits — l'index git et `third_party/UPSTREAM.md` — et il aurait fallu une
regle pour departager en cas de desaccord. Deux sources de verite pour une meme
valeur : c'est exactement le defaut qu'on refuse ailleurs dans ce depot.

Le submodule n'apporte par ailleurs rien que le couple manifeste + script n'ait
deja : la reproductibilite vient du SHA, pas du mecanisme qui le porte. Il coute
en revanche un `.gitmodules`, un `--recursive` que personne ne tape, et une
configuration de CI supplementaire.

**Retenu : un SHA unique, epingle dans un fichier texte, consomme par un script.**

    third_party/UPSTREAM.md        LE SHA, sa date, la raison de la derniere montee
    third_party/ladybird/          arbre recupere (ignore par git)
    tools/ladybird/fetch.sh        recuperation + verification (`--verifie`)
    tools/ladybird/patches/        nos divergences, une par fichier, numerotees

Les divergences vivent en patches **separes** et non en modifications directes du
submodule : c'est ce qui permet de dire, a tout moment, ce que nous avons change
par rapport a upstream — et de proposer en amont ce qui merite de l'etre.

Regle : tout patch qui n'est pas specifique a Bouchaud doit etre soumis en amont
plutot que conserve.
