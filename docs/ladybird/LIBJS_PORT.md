# Portage de LibJS sur Bouchaud OS

## Etat actuel

Le moteur JavaScript du navigateur Bouchaud est **QuickJS**, appele depuis
`tools/userland/navigateur/bojs.cpp` (28 Ko) et pilote par le moteur Python
(`moteur/js.py`, 100 Ko). Il fonctionne et reste le chemin par defaut.

LibJS **est construit et s'execute**, sur l'hote et en ring 3 sous QEMU — voir
« Le jalon M4 est atteint » plus bas. Il n'est pas encore branche au navigateur :
`BO_WEB_ENGINE=ladybird` reste a cabler, et QuickJS demeure le chemin par
defaut.

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
| ICU | — | **fait** | 77.1 statique, sans `dlopen` |
| libtommath | — | **fait** | 1.3.0, version epinglee |
| Caisses Rust (5) | — | **fait** | archives + en-tetes FFI |
| LibUnicode | 23 | **fait** | 8 verifications (donnees ICU) |
| LibCrypto, sous-ensemble | 3 | **fait** | 9 verifications (BigInt/BigFraction) |
| LibJS + satellites | 275 | **fait** | 12 verifications, hote **et** ring 3 |

### La lecon de ce lot : c'est la version, pas le code

Trois fois de suite, ce qui a bloque n'etait ni l'ABI, ni le ring 3, ni une
incompatibilite de conception — c'etait la version d'une bibliotheque tierce
prise dans la distribution au lieu d'etre construite depuis la reference
d'upstream.

| Bibliotheque | Distribution | `vcpkg.json` | Consequence |
|---|---|---|---|
| ICU | 74 | 78.3 | 4 des 23 sources de LibUnicode ne compilent pas |
| libtommath | 1.2.1 | 1.3.0 | 2 des 3 sources de LibCrypto ne compilent pas |
| OpenSSL | 3.0.13 | 3.6.3 | `PK/MLKEM.cpp`, `PK/MLDSA.cpp` : API post-quantiques absentes |

Le detail d'ICU merite d'etre retenu : `vcpkg.json` epingle **78.3**, mais aucun
tag `release-78-*` n'existe sur le depot public d'ICU — le plus recent y est
`release-77-1`. La reference de vcpkg ne designe donc aucune etiquette amont
accessible. `release-77-1` compile les quatre fichiers et devient la version
retenue ; l'ecart est assume, pas ignore.

La regle qui en decoule, appliquee desormais : **on construit ce que
`vcpkg.json` epingle, et on ne retient une version de la distribution qu'apres
l'avoir vue passer.** Aucun repli silencieux vers le systeme : `build-libunicode.sh`
refuse de demarrer si `third_party/icu-hote` est absent, plutot que de retomber
sur ICU 74 et de faire reapparaitre les memes quatre echecs un mois plus tard.

### OpenSSL : une dette notee, pas payee

LibCrypto complet exige OpenSSL ≥ 3.5. La mesure des inclusions montre que LibJS
n'en a pas besoin : il ne prend de LibCrypto que quatre en-tetes, tous du cote
arithmetique.

    11  LibCrypto/BigInt/SignedBigInteger.h     -> le type BigInt du langage
     5  LibCrypto/BigFraction/BigFraction.h     -> conversions numeriques exactes
     4  LibCrypto/BigInt/UnsignedBigInteger.h
     2  LibCrypto/Forward.h

Rien dans `Cipher/`, `Hash/`, `PK/`, `Certificate/` ou `ASN1/` n'est atteint
depuis LibJS. On construit donc trois sources au lieu de vingt-neuf, et OpenSSL
redevient necessaire avec **LibTLS et RequestServer** — c'est-a-dire avec le
reseau, pas avec le moteur JavaScript.

### Ladybird est un projet a deux chaines de compilation

Ce n'est pas un detail d'organisation, et cela se decouvre tard si on ne le
cherche pas : `Cargo.toml` a la racine declare un espace de travail de dix
caisses Rust, dont **cinq** sont sur le chemin de LibJS. `tools/ladybird/build-rust.sh`
les construit toutes.

Deux pieges y sont documentes parce qu'ils coutent tous les deux une heure :

- Bouchaud epingle sa cible noyau dans `.cargo/config.toml` a la racine ; toute
  invocation de cargo depuis l'arbre en herite. Sans `CARGO_BUILD_TARGET`
  explicite, cargo construit les caisses de Ladybird pour le noyau et echoue sur
  un message qui ne parle ni de Ladybird ni de cible.
- Une invocation de cargo **par caisse**, jamais une seule pour toutes.
  `libunicode_rust` est a la fois `staticlib` et `rlib`, donc dependance de
  `libregex_rust` ; dans une invocation unique, cargo unifie les fonctionnalites
  et la fonctionnalite `allocator` se retrouve declaree deux fois — un seul
  `#[global_allocator]` etant permis par unite de liaison. Le CMake d'upstream
  fait un `import_rust_crate` a la fois : c'etait une condition de correction,
  pas une preference de style.

## Le jalon M4 est atteint

Sous QEMU, en ring 3, sur Bouchaud OS :

    == temoin LibJS ==
      ok     un royaume JS est construit
             1 + 2 = 3
             10! = 3628800
             2n ** 64n = 18446744073709551616
             'e+U+0301'.normalize('NFC') = U+e9
             Intl.NumberFormat('fr-FR') = fr-FR
             JSON aller-retour = {"b":[1,2],"a":true}
             regex = 15/08/2026
             try/catch = TypeError
             200 000 objets abandonnes, garde = intact
             --- console.log ---
             "Hello Bouchaud"
    RESULTAT : 0 verification(s) en echec (12 passees)

## Ce que le ring 3 a exige du noyau, et que l'hote ne montrait pas

La construction ne prouvait rien sur Bouchaud. Les douze memes verifications
passaient sur l'hote depuis le premier essai ; sous QEMU, la sonde mourait —
quatre fois de suite, pour quatre raisons distinctes, aucune visible a la
compilation.

### 1. `/proc/self/maps` — sans quoi rien de Ladybird ne demarre

`AK::StackInfo` appelle `pthread_getattr_np()`, qui pour le **thread principal**
lit `/proc/self/maps` : c'est le noyau qui a pose cette pile, et rien dans l'ABI
ne la lui communique autrement. Le fichier n'existait pas ; la glibc rendait
`ENOENT` et AK faisait `VERIFY_NOT_REACHED()`.

Cela arretait **toute** brique de Ladybird — AK, LibGC, LibJS. Et pour LibGC ce
n'est pas qu'une question de demarrage : son ramasse-miettes est
**conservateur** et balaie la pile a la recherche de racines. Des bornes fausses
ne le feraient pas planter, elles lui feraient recolter des objets vivants.

Le fichier est desormais produit par processus (`FdKind::Instantane`), et non
ecrit dans le RAMFS — `/proc/self` y serait partage par tout le systeme, donc
faux pour tout le monde sauf un.

### 2. Un espace d'adressage de 512 Gio ne suffit plus

`GC::PrimitiveStorage` reserve **4 Tio** des la construction de la `JS::VM`,
pour que ses pointeurs compresses partagent leurs bits de poids fort.
`GC::BlockAllocator` en reserve **4 Tio** de plus, pour la meme raison appliquee
aux blocs du tas. Un processus LibJS reserve donc 8 Tio d'espace d'adressage
avant d'avoir alloue le moindre octet utile.

Le creneau utilisateur de Bouchaud etait **une** entree PML4 : 512 Gio, huit
fois moins que la premiere de ces deux reservations. Il en compte desormais 64,
soit 32 Tio, et la pile est remontee au sommet — laissee au milieu, elle aurait
coupe les reservations en deux.

Ce n'est pas propre a Ladybird : la compression de pointeurs par cage est la
technique courante des moteurs JavaScript modernes.

### 3. Reserver n'est pas allouer

`mmap(PROT_NONE)` demande une **plage d'adresses**, pas de la memoire. Bouchaud
allouait avidement : la reservation de 4 Tio demandait 4 Tio de RAM et rendait
`ENOMEM`. Symetriquement, `decommit_memory()` de LibCore **rend** la memoire par
un `mmap(PROT_NONE, MAP_FIXED)` — s'en tenir a rendre l'adresse faisait de la
liberation une operation sans effet.

### 4. La pagination a la demande

Le dernier obstacle, et le plus instructif. mimalloc — l'allocateur d'AK, donc
de tout Ladybird — prend ses arenes **par gibioctet**, en lecture-ecriture, avec
`MAP_NORESERVE`, et n'en touche qu'une fraction. Les peupler a l'appel epuisait
les 2 Gio de la machine en deux arenes ; les refuser aurait fait echouer
l'allocateur.

La seule reponse juste est celle de Linux : noter la promesse, et n'allouer
chaque page qu'a son premier acces. Le gestionnaire de faute de page consulte
desormais les plages promises du processus et peuple la page fautive avec les
droits enregistres a la reservation. Une faute **hors** de toute promesse reste
une faute : sans cette rigueur, le noyau transformerait chaque dereference de
pointeur nul en allocation silencieuse.

### 5. `clone3`

La glibc l'emet avant `clone`. C'est le meme piege que celui deja rencontre avec
`fork` : l'appel systeme historique etait implemente et teste, mais les
programmes reels emettent l'autre. Une difference compte — `struct clone_args`
donne la pile par sa **base et sa taille**, la ou `clone` recevait son
**sommet**.

## Une lecon de methode : le verdict qui mentait

`tools/test.sh` comptait `ak-probe` et `libgc-probe` au vert alors qu'ils
**mouraient** en ring 3 sur `StackInfo.cpp`. La banniere `== temoin AK ==`
s'imprime avant le premier appel fautif ; le verdict cherchait cette banniere,
la trouvait, et concluait au succes. Les lignes « RESULTAT » qui suivaient dans
le journal appartenaient a d'autres sondes.

Le verdict exige desormais que le bilan d'une sonde **suive sa banniere**, sans
qu'une autre banniere ne s'intercale, et annonce zero echec. C'est la seule
lecture qu'un plantage ne peut pas satisfaire.

## Ce qui reste

Le moteur JavaScript tourne ; le moteur **web** n'existe pas encore. La suite est
LibIPC, LibGfx, puis LibWeb — et `BO_WEB_ENGINE=ladybird` cote navigateur.

### La facture d'ICU, connue d'avance

`libicudata.a` fait **31 Mio**, en un seul objet — l'edition de liens le prend
donc en entier ou pas du tout. Le temoin LibUnicode pese 37 Mio pour cette
raison, et le userland actuel fait deja 52 Mio.

Si cette taille devient inacceptable pour l'image, la reponse est
`ICU_DATA_FILTER_FILE`, qui restreint le jeu de donnees construit. Ce n'est pas
fait : on mesurera d'abord ce dont LibJS a reellement besoin. Le temoin
`libunicode-probe` existe precisement pour ce moment-la — il verifie les
**donnees** (normalisation, `likelySubtags`, ruptures de graphemes) et non les
signatures, parce qu'un jeu de donnees ampute ne casse rien a la compilation et
ne leve aucune erreur : il rend des reponses fausses mais plausibles.

## Critere de succes du document

Le portage est reussi quand, sous QEMU :

    [bo] moteur : ladybird (LibJS)
    Hello Bouchaud

et que `BO_WEB_ENGINE=legacy` continue de donner le navigateur actuel intact.
