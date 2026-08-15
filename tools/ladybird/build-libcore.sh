#!/bin/bash
# Construit LibSync, LibCore **complet** et LibThreading, et joue un temoin.
#
#   ./tools/ladybird/build-libcore.sh          hote
#   ./tools/ladybird/build-libcore.sh --cible  Bouchaud (static-pie)
#
# ## Ce script construisait un sous-ensemble ; il ne le fait plus
#
# Jusqu'au jalon M4, il produisait `libCoreMin.a` : dix-sept sources sur
# vingt-huit, choisies par la mesure des inclusions de LibGC et LibJS, puis
# etendues au fur et a mesure que l'editeur de liens reclamait.
#
# C'etait la bonne facon de **demarrer** — construire LibCore en entier exigeait
# LibUnicode, LibURL et LibTextCodec, donc ICU, donc tout le tiers-parti, avant
# meme de savoir si dix-sept fichiers compilaient. Ce n'etait pas une facon de
# **rester** : un sous-ensemble fige est un fork simplifie qui ne dit pas son
# nom, et qui diverge un peu plus a chaque montee de SHA.
#
# Ces trois bibliotheques existent maintenant. Le graphe reel est donc
# atteignable, et c'est lui qu'on construit : les **quarante-deux** sources que
# le CMake d'upstream retient pour une cible POSIX/Linux. Aucune n'a demande la
# moindre modification de source Ladybird.
#
# ## La selection de plateforme, telle qu'upstream la fait
#
# `Libraries/LibCore/CMakeLists.txt` choisit ses sources par des `if()` :
# Windows contre POSIX, `sys/inotify.h` present ou non, Linux contre Apple pour
# les statistiques de processus et la surveillance du fuseau horaire. Ce script
# fait les memes choix, explicitement, en nommant la capacite Bouchaud qui les
# motive — c'est la « couche plateforme » du portage, et elle vit ici plutot que
# dans une copie modifiee de l'arbre amont.

set -eu
cd "$(dirname "$0")/../.."
RACINE=$(pwd)

LB="$RACINE/third_party/ladybird"
CIBLE=hote
FLAGS_CIBLE=""
if [ "${1:-}" = "--cible" ]; then
    CIBLE=bouchaud
    FLAGS_CIBLE="-static-pie -fPIE"
fi

DEPS="$RACINE/third_party/deps-$CIBLE"
AK="$RACINE/third_party/build-ak-$CIBLE"
GEN="$RACINE/third_party/gen-$CIBLE"
ICU="$RACINE/third_party/icu-$CIBLE"
SORTIE="$RACINE/third_party/build-libcore-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

[ -f "$AK/libAK.a" ] || { rouge "AK absent — lancer build-ak.sh ${1:-}"; exit 1; }
[ -d "$GEN/LibCore" ] || { rouge "en-tetes absents — lancer build-entetes.sh ${1:-}"; exit 1; }
[ -d "$ICU/include" ] || { rouge "ICU absent — lancer build-icu.sh ${1:-}"; exit 1; }

CXX=${CXX:-clang++}
# `-fno-rtti` n'est **pas** pose : upstream ne desactive que les exceptions
# (Meta/CMake/compile_options.cmake). `AK/TypeCasts.h` s'appuie sur
# `dynamic_cast`, et melanger des unites compilees avec et sans RTTI produit des
# transtypages qui echouent a l'execution sans rien dire.
# `-DAK_SYSTEM_CACHE_ALIGNMENT_SIZE=64` : voir build-ak.sh.
CXXFLAGS="-std=c++23 -O2 -fno-exceptions -fPIC -DAK_SYSTEM_CACHE_ALIGNMENT_SIZE=64 $FLAGS_CIBLE \
    -I$LB -I$LB/Libraries -I$AK/gen -I$GEN -I$DEPS/include -I$ICU/include \
    -Wno-unused-parameter -Wno-unknown-pragmas -Wno-invalid-constexpr \
    -Wno-unqualified-std-cast-call -Wno-user-defined-literals \
    -Wno-unknown-warning-option"

mkdir -p "$SORTIE/obj"

# --- LibSync : les primitives de synchronisation ----------------------------
#
# Trois fichiers, aucune dependance hors AK. LibGC en depend publiquement.
SRC_SYNC="MutexPOSIX.cpp ConditionVariablePOSIX.cpp RWLockPOSIX.cpp"

# --- LibCore : la liste d'upstream, sans retrait ----------------------------
#
# Lue dans le `set(SOURCES ...)` du CMakeLists, pour que l'ajout d'un fichier en
# amont soit pris sans que rien ne soit a modifier ici.
SRC_CORE=$(sed -n '/^set(SOURCES/,/^)/p' "$LB/Libraries/LibCore/CMakeLists.txt" \
           | grep -oE '[A-Za-z0-9_]+\.cpp')

# --- Les choix de plateforme ------------------------------------------------
#
# Un par `if()` du CMakeLists amont, dans le meme ordre, avec la raison.

# Geolocalisation : `GeolocationProviderCoreLocation.mm` est le chemin Apple.
# Bouchaud n'a pas de service de localisation — upstream fournit lui-meme la
# variante « non implementee », et c'est elle qu'il faut prendre plutot que d'en
# ecrire une.
SRC_CORE="$SRC_CORE GeolocationProviderUnimplemented.cpp"

# POSIX contre Windows. Bouchaud expose une ABI Linux : la branche `else()`.
SRC_CORE="$SRC_CORE AnonymousBuffer.cpp EventLoopImplementationUnix.cpp \
    FilePosix.cpp LocalServer.cpp MappedFile.cpp Process.cpp Socket.cpp \
    System.cpp TCPServer.cpp UDPServer.cpp"

# Surveillance de fichiers : `check_include_file(sys/inotify.h)` chez upstream.
# Bouchaud n'implemente pas inotify — aucun `inotify_init`, `inotify_add_watch`
# ni `inotify_rm_watch` dans `src/kernel/abi/`. La variante « non implementee »
# rend `ENOTSUP` proprement ; la variante inotify se lierait a des appels
# systeme absents et echouerait a l'execution, pas a la liaison.
SRC_CORE="$SRC_CORE FileWatcherUnimplemented.cpp"

# Statistiques de processus. `ProcessStatisticsLinux.cpp` lit `/proc` — ce que
# Bouchaud fournit (voir `src/kernel/sysroot.rs`). On prend donc bien la
# variante Linux, et non celle « non implementee » : c'est le sens de la
# compatibilite d'ABI qu'on construit.
SRC_CORE="$SRC_CORE Platform/ProcessStatisticsLinux.cpp"

# Fuseau horaire : la branche `LINUX OR BSD`.
SRC_CORE="$SRC_CORE TimeZoneWatcherUnix.cpp"

# --- LibThreading -----------------------------------------------------------
SRC_THREADING="Thread.cpp"

compile() {
    local dossier=$1 src=$2
    local obj="$SORTIE/obj/$(basename "$dossier")-$(echo "${src%.cpp}" | tr / -).o"
    OBJETS="$OBJETS $obj"
    if [ -f "$obj" ] && [ "$obj" -nt "$LB/Libraries/$dossier/$src" ]; then
        return 0
    fi
    local log="${obj%.o}.log"
    if ! $CXX $CXXFLAGS -c "$LB/Libraries/$dossier/$src" -o "$obj" 2> "$log"; then
        rouge "  ECHEC $dossier/$src"
        grep -m3 "error:" "$log" | sed 's/^/          /'
        ECHECS=$((ECHECS + 1))
    fi
}

ECHECS=0
OBJETS=""

info "== LibSync ($CIBLE) : $(echo "$SRC_SYNC" | wc -w) fichiers =="
for src in $SRC_SYNC; do compile LibSync "$src"; done

info "== LibCore ($CIBLE) : $(echo "$SRC_CORE" | wc -w) fichiers =="
for src in $SRC_CORE; do compile LibCore "$src"; done

info "== LibThreading ($CIBLE) : $(echo "$SRC_THREADING" | wc -w) fichiers =="
for src in $SRC_THREADING; do compile LibThreading "$src"; done

if [ "$ECHECS" -gt 0 ]; then
    rouge "$ECHECS fichier(s) n'ont pas compile"
    exit 1
fi

ar rcs "$SORTIE/libCore.a" $OBJETS
vert "  ok    libCore.a ($(du -h "$SORTIE/libCore.a" | cut -f1))"

info "  CXX   temoin LibCore"
# LibCore complet se lie a LibUnicode, LibURL et LibTextCodec — c'est ce que dit
# `target_link_libraries` en amont, et c'est desormais vrai chez nous aussi.
UNI="$RACINE/third_party/build-libunicode-$CIBLE"
JS="$RACINE/third_party/build-libjs-$CIBLE"
RUSTLIB="$RACINE/third_party/build-rust-$CIBLE/x86_64-unknown-linux-gnu/release"
# ## L'amorcage, et pourquoi le temoin peut attendre
#
# LibCore complet **reference** LibUnicode, LibURL et LibTextCodec. Or LibURL et
# LibTextCodec sont construits avec LibJS, qui a besoin de LibCore. Au niveau des
# archives, la dependance est donc circulaire.
#
# Elle ne l'est pas au niveau des objets : compiler LibCore n'exige que des
# en-tetes, et compiler LibUnicode n'exige pas `libCore.a`. Seule **l'edition de
# liens** d'un executable a besoin de tout le monde.
#
# D'ou la marche a suivre, que `build-tout.sh` applique : produire les archives
# dans l'ordre, puis relier les temoins a la fin. Sur un arbre neuf, ce script
# s'arrete donc apres `libCore.a` en le disant, plutot que d'echouer sur une
# edition de liens qui ne peut pas encore aboutir.
if [ ! -f "$UNI/libUnicode.a" ] || [ ! -f "$JS/libJS.a" ]; then
    info "  note  LibUnicode/LibJS pas encore construits : temoin remis a plus tard"
    info "        (relancer ce script apres build-libjs.sh, ou passer par build-tout.sh)"
    exit 0
fi
SUP="$JS/libJS.a $UNI/libUnicode.a \
     $RUSTLIB/liblibunicode_rust.a $RUSTLIB/liblibregex_rust.a \
     $RUSTLIB/libliburl_rust.a $RUSTLIB/liblibjs_rust.a \
     $RUSTLIB/liblibtextcodec_rust.a"

# `--start-group` / `--end-group` : les archives sont **mutuellement
# recursives**. Les caisses Rust rappellent le C++ (`libunicode_rust` appelle
# `unicode_property_matches`, fourni par LibUnicode), et LibUnicode appelle le
# Rust en retour. Un editeur de liens ne revient pas en arriere : quel que soit
# l'ordre choisi, l'une des deux directions reste non resolue. Le groupe le fait
# reparcourir jusqu'a stabilite — c'est la reponse prevue pour ce cas, et non un
# contournement.
$CXX $CXXFLAGS "$RACINE/tools/ladybird/libcore-probe.cpp" -o "$SORTIE/libcore-probe" \
    -Wl,--start-group \
    "$SORTIE/libCore.a" $SUP "$AK/libAK.a" \
    -Wl,--end-group \
    -Wl,--allow-multiple-definition \
    -L"$DEPS/lib" -lsimdjson -ltommath -lpsl -lfmt -lsimdutf -lmimalloc \
    -L"$ICU/lib" -licui18n -licuuc -licudata \
    -lpthread -lm -ldl

vert "temoin construit : $SORTIE/libcore-probe"
if [ "$CIBLE" = "hote" ]; then
    echo
    "$SORTIE/libcore-probe"
fi
