# Portage de LibJS sur Bouchaud OS

## Etat actuel

Le moteur JavaScript du navigateur Bouchaud est **QuickJS**, appele depuis
`tools/userland/navigateur/bojs.cpp` (28 Ko) et pilote par le moteur Python
(`moteur/js.py`, 100 Ko). Il fonctionne et reste le chemin par defaut.

LibJS n'est pas encore construit, ni sur l'hote, ni pour la cible.

## Objectif

`BO_WEB_ENGINE=ladybird` execute du JavaScript par LibJS, en ring 3 sous
Bouchaud OS, avec GC, exceptions JS, promesses et microtaches.

## Le vrai obstacle

Ce n'est ni l'ABI ni le ring 3 : c'est la **cloture de liaison** (voir
`DEPENDENCIES.md` §2). `LibJS` tire `LibUnicode` en **public**, qui tire ICU 78.3
et une bibliotheque Rust. `LibCrypto` tire OpenSSL et libtommath.

    hello.js  ->  LibJS  ->  LibGC  ->  LibCore  ->  musl  ->  ABI Bouchaud
                     |
                     +-> LibUnicode -> ICU (i18n + uc + data) + Rust
                     +-> LibCrypto  -> OpenSSL::Crypto + libtommath
                     +-> simdjson, fast-float, fmt

Toute estimation qui ignore cette colonne est fausse. L'ordre des etapes ci-dessous
la place donc en premier.

## Etapes

### Etape 0 — chaine de compilation (prerequis, aucun code Ladybird)

Avant AK, il faut prouver qu'on sait produire un binaire **C++23** statique-PIE
pour Bouchaud.

- Verifier la version de `musl-g++` disponible : C++23 exige GCC ≥ 13.
- Compiler un programme temoin C++23 (concepts, `std::expected`, `if consteval`)
  en `-static-pie -fno-exceptions`, et l'executer sous QEMU.
- Critere de succes : le temoin s'execute et sort 0.

Si `musl-gcc` du systeme est trop ancien, construire une chaine croisee dediee
devient la premiere PR reelle du chantier.

### Etape 1 — AK sur l'hote

AK ne depend de rien (196 fichiers, 0 dependance). C'est la brique d'entree.

- Compiler AK pour l'hote Linux avec le CMake d'upstream.
- Executer `Tests/AK`.
- Critere : suite AK au vert sur l'hote.

### Etape 2 — AK pour la cible Bouchaud

- Toolchain file CMake `Meta/CMake/Bouchaud.cmake` (ou equivalent hors arbre).
- Critere : `libAK.a` produit ; un `main()` qui utilise `AK::String` et
  `AK::Vector` s'execute en ring 3 et imprime sur la serie.

C'est le premier vrai test de l'ABI : AK utilise `mmap`, `pthread` (27),
`clock_gettime`, `getrandom` — tous presents.

### Etape 3 — LibCore minimal

LibJS et LibGC n'incluent que **neuf** en-tetes de LibCore, mesures :

| En-tete | Inclusions | Necessaire pour `1+2` ? |
|---|---|---|
| `LibCore/ImmutableBytes.h` | 6 | oui |
| `LibCore/System.h` | 3 | oui |
| `LibCore/Forward.h` | 3 | oui |
| `LibCore/ElapsedTimer.h` | 3 | oui |
| `LibCore/Timer.h` | 2 | non (timers JS) |
| `LibCore/StandardPaths.h` | 1 | non |
| `LibCore/File.h` | 1 | non |
| `LibCore/EventLoop.h` | 1 | non (promesses/microtaches) |
| `LibCore/Environment.h` | 1 | non |

Plus `LibSync/Mutex.h` et `LibSync/ConditionVariable.h` (1 chacun).

Strategie : ne pas porter LibCore en entier. Construire la cible CMake avec le
sous-ensemble compilable, et ajouter au fur et a mesure que l'editeur de liens
reclame. La couche plateforme est decrite dans `BOUCHAUD_PLATFORM.md`.

### Etape 4 — tiers-parti

Dans cet ordre, du plus simple au plus lourd :

1. `fmt`, `fast-float`, `simdjson` — en-tetes/sources portables, peu de systeme.
2. `libtommath` — arithmetique pure.
3. `OpenSSL::Crypto` — uniquement `libcrypto`, pas `libssl`. Sous-ensemble sans
   moteur ni chargement dynamique.
4. `ICU` — le morceau. Decider entre : ICU complet avec ses donnees (~30 Mio),
   ou construction avec un jeu de donnees restreint (`ICU_DATA_FILTER_FILE`).
   Le userland actuel fait deja 52 Mio ; la question de la taille est reelle.
5. `libunicode_rust` — exige une cible Rust supplementaire pour Bouchaud.

Chaque paquet est une PR avec son fichier de licence dans `THIRD_PARTY_LICENSES/`.

### Etape 5 — LibGC

- LibGC utilise `mmap` une fois (le tas) et `pthread` via LibSync/LibThreading.
- Critere : allouer, provoquer une collecte, verifier qu'un objet non reference
  est bien recolte, sous QEMU.

Point de vigilance : LibGC balaie les piles a la recherche de racines. Il doit
connaitre les bornes de pile de chaque thread. Sur Bouchaud, `clone` fixe la pile
du thread ; verifier que `pthread_getattr_np` de musl rend des bornes correctes,
sinon le GC recoltera des objets vivants — un defaut qui ne se voit pas tout de
suite et corrompt plus tard.

### Etape 6 — LibJS, `1 + 2`

Petit executable qui construit un `JS::Realm`, evalue `1 + 2`, imprime le
resultat. D'abord sur l'hote, puis en croisant.

### Etape 7 — `console.log("Hello Bouchaud")` en ring 3

Critere : la ligne apparait sur la sortie serie sous QEMU. **C'est le jalon M4**,
celui qui prouve toute la chaine.

### Etape 8 — semantique complete

Allocations, GC sous pression, exceptions JS, `Promise`, microtaches, timers
minimum, Unicode (`Intl` de base). Chaque point avec un cas de test.

Puis Test262 : d'abord un sous-ensemble hote, ensuite sous QEMU si le temps
d'execution le permet.

## Coexistence avec le moteur actuel

    lanceur (/bo-navigateur)
        |
        +-- BO_WEB_ENGINE=legacy (defaut) --> Python + QuickJS
        |
        +-- BO_WEB_ENGINE=ladybird --------> port Ladybird

Le drapeau est lu au demarrage, journalise, et le repli sur `legacy` est
automatique si le binaire Ladybird est absent. Aucune suppression du moteur
actuel avant M5.

## Risques

| Risque | Gravite | Attenuation |
|---|---|---|
| ICU trop volumineux pour l'image | elevee | filtre de donnees ICU ; mesurer avant de porter |
| `musl-g++` trop ancien pour C++23 | elevee | etape 0 avant tout ; sinon chaine croisee dediee |
| Bornes de pile fausses -> GC corrompt | elevee | test dedie des l'etape 5 |
| Cible Rust pour `libunicode_rust` | moyenne | evaluer si LibUnicode compile sans, au prix de fonctions |
| OpenSSL suppose `/dev/urandom`, `getrandom` | faible | les deux existent |
| Divergence upstream par patches locaux | moyenne | patches numerotes, proposes en amont |

## Etat mesure au 2026-08-15 (PR 1-8)

Ce qui est construit et verifie, sur l'hote **et** pour la cible `-static-pie` :

| Brique | Fichiers | Etat | Temoin |
|---|---|---|---|
| Chaine C++23 | — | **fait** | 17 verifications |
| fast_float, fmt, simdutf, mimalloc | — | **fait** | versions de `vcpkg.json` |
| AK | 39 | **fait** | 13 verifications |
| LibSync | 3 | **fait** | 6 verifications (dont 4 fils en contention) |
| LibCore, sous-ensemble | 17 | **fait** | idem |
| LibThreading | 1 | **fait** | idem |
| LibGC | 20 | **fait** | 5 verifications (513 cellules recoltees) |
| LibUnicode | 23 | **bloque** | voir ci-dessous |
| LibCrypto, LibJS | 556+ | non commence | — |

### Le blocage : la version d'ICU

Sur 23 fichiers de LibUnicode, **19 compilent** contre l'ICU 74 de la
distribution. Les 4 restants echouent sur des differences d'API entre ICU 74 et
la version qu'upstream emploie :

- `CharacterTypes.cpp` : `icu::UnicodeSet::strings()` est **privee** en 74,
  publique a partir de 77 ;
- `Calendars/AdjustedEraCalendar.h` et `ChineseDangiCalendar.h` : les signatures
  virtuelles d'`icu::Calendar` ont change ; les `override` ne recouvrent plus
  rien.

Aucun code a nous ne corrige cela : c'est upstream qui emploie des API recentes.

`vcpkg.json` epingle **ICU 78.3**, mais aucun tag `release-78-*` n'existe sur le
depot public d'ICU — le plus recent y est `release-77-1`. La version de vcpkg ne
correspond donc pas a une etiquette amont accessible. `release-77-1` expose bien
`strings()` en public et est la cible retenue.

### Ce qui reste, dans l'ordre

1. ICU 77 construit en statique (en cours).
2. LibUnicode : reprendre les 4 fichiers avec ICU 77.
3. LibCrypto : OpenSSL 3.0 de la distribution + libtommath.
4. LibTextCodec, LibRegex, LibSyntax, LibFileSystem, LibURL — les dependances
   privees de LibJS que l'editeur de liens reclamera.
5. LibJS : 556 fichiers.
6. Le temoin `1 + 2`, puis `console.log`.

Rien dans ce qui precede n'est un obstacle de conception : ce sont des
constructions a enchainer. Le seul inconnu restant est le temps de compilation de
LibJS.

## Critere de succes du document

Le portage est reussi quand, sous QEMU :

    [bo] moteur : ladybird (LibJS)
    Hello Bouchaud

et que `BO_WEB_ENGINE=legacy` continue de donner le navigateur actuel intact.
