#!/bin/bash
# Construit LibSync et le sous-ensemble de LibCore dont LibGC et LibJS ont besoin.
#
#   ./tools/ladybird/build-libcore.sh          hote
#   ./tools/ladybird/build-libcore.sh --cible  Bouchaud (static-pie)
#
# ## Pourquoi un sous-ensemble
#
# `LibCore` complet se lie a `LibUnicode` (donc ICU), `LibURL` et `LibTextCodec`.
# Or la mesure d'inclusions (docs/ladybird/DEPENDENCIES.md) dit que LibGC et
# LibJS n'utilisent que **neuf** en-tetes de LibCore. Construire les 28 sources
# et leurs dependances avant de savoir si les neuf compilent serait payer ICU
# pour rien.
#
# On construit donc ce que ces neuf en-tetes reclament, et **l'editeur de liens
# dit le reste** : chaque symbole manquant nomme le fichier a ajouter. C'est plus
# sur qu'une liste devinee, et la liste ci-dessous est exactement ce que cette
# methode a produit — pas ce qu'on esperait.
#
# ## Ce qui est volontairement absent
#
# Rien de ce qui touche au reseau, aux processus, aux fichiers surveilles ou aux
# fuseaux horaires. Ces morceaux viendront quand LibIPC et RequestServer les
# reclameront, avec leurs dependances.

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
SORTIE="$RACINE/third_party/build-libcore-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

[ -f "$AK/libAK.a" ] || { rouge "AK absent — lancer build-ak.sh ${1:-}"; exit 1; }

CXX=${CXX:-clang++}
# `-fno-rtti` n'est **pas** pose : upstream ne desactive que les exceptions
# (Meta/CMake/compile_options.cmake). `AK/TypeCasts.h` s'appuie sur
# `dynamic_cast`, et melanger des unites compilees avec et sans RTTI produit des
# transtypages qui echouent a l'execution sans rien dire.
CXXFLAGS="-std=c++23 -O2 -fno-exceptions -fPIC $FLAGS_CIBLE \
    -I$LB -I$LB/Libraries -I$AK/gen -I$SORTIE/gen -I$DEPS/include \
    -Wno-unused-parameter -Wno-unknown-pragmas -Wno-invalid-constexpr \
    -Wno-unqualified-std-cast-call -Wno-user-defined-literals \
    -Wno-unknown-warning-option"

mkdir -p "$SORTIE/obj" "$SORTIE/gen"

# --- En-tetes d'export generes ----------------------------------------------
#
# `ladybird_lib(... EXPLICIT_SYMBOL_EXPORT)` appelle `generate_export_header`,
# qui produit un `Export.h` par bibliotheque definissant `SYNC_API`, `CORE_API`…
# Ces macros pilotent la visibilite des symboles d'une bibliotheque **partagee**.
#
# Nous produisons des archives statiques : la visibilite ne se pose pas, et les
# macros doivent donc etre vides. C'est exactement ce que CMake genere quand
# `<NOM>_STATIC_DEFINE` est defini — on ecrit le meme resultat directement.
genere_export() {
    local biblio=$1 macro=$2
    local fichier="$SORTIE/gen/$biblio/Export.h"
    mkdir -p "$SORTIE/gen/$biblio"
    [ -f "$fichier" ] && return 0
    info "  GEN   $biblio/Export.h"
    cat > "$fichier" <<EXPORT
/* Genere par tools/ladybird/build-libcore.sh.
 *
 * Equivalent de ce que \`generate_export_header\` produit pour une
 * bibliotheque statique : les macros de visibilite sont vides, parce qu'une
 * archive n'exporte rien — tout est resolu a l'edition de liens finale. */
#pragma once
#define ${macro}
#define ${macro%_API}_NO_EXPORT
EXPORT
}

genere_export LibSync SYNC_API
genere_export LibCore CORE_API
genere_export LibThreading THREADING_API

# --- LibSync : trois fichiers, aucune dependance hors AK --------------------
#
# Les primitives de synchronisation, au-dessus de pthread. C'est la brique la
# plus simple de tout Ladybird, et LibGC en depend publiquement.
SRC_SYNC="MutexPOSIX.cpp ConditionVariablePOSIX.cpp RWLockPOSIX.cpp"

# --- LibCore : le sous-ensemble ---------------------------------------------
#
# Les neuf en-tetes reclames par LibGC/LibJS et leur cloture de symboles.
# LibGC reclame en plus : File, StandardPaths, System, Timer. Ils entrainent
# EventLoop et ses satellites — c'est l'editeur de liens qui l'a dit, pas une
# supposition.
# `FilePosix.cpp` et `MappedFile.cpp` : ajoutes quand l'edition de liens de
# LibJS les a reclames. `File.cpp` ne porte que la partie portable — les
# virtuelles (`stat`, `fstat`, `truncate`, `open_path`) et donc la **table
# virtuelle** de `Core::File` vivent dans la variante POSIX. Sans elle, le
# defaut ne se manifeste pas a la compilation mais par « undefined reference to
# vtable for Core::File », un message qui ne nomme aucun fichier.
SRC_CORE="ElapsedTimer.cpp Environment.cpp ImmutableBytes.cpp \
    File.cpp FilePosix.cpp MappedFile.cpp StandardPaths.cpp System.cpp Timer.cpp \
    EventLoop.cpp EventLoopImplementation.cpp EventLoopImplementationUnix.cpp \
    EventReceiver.cpp Notifier.cpp ThreadEventQueue.cpp SocketAddress.cpp \
    DirIterator.cpp DirectoryEntry.cpp Directory.cpp AddressInfoVector.cpp"

# LibThreading : LibGC s'en sert pour son fil de collecte.
SRC_THREADING="Thread.cpp"

compile() {
    local dossier=$1 src=$2
    local obj="$SORTIE/obj/$(basename "$dossier")-${src%.cpp}.o"
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

info "== LibSync ($CIBLE) =="
for src in $SRC_SYNC; do compile LibSync "$src"; done

info "== LibCore, sous-ensemble ($CIBLE) =="
for src in $SRC_CORE; do compile LibCore "$src"; done

info "== LibThreading ($CIBLE) =="
for src in $SRC_THREADING; do compile LibThreading "$src"; done

if [ "$ECHECS" -gt 0 ]; then
    rouge "$ECHECS fichier(s) n'ont pas compile"
    exit 1
fi

ar rcs "$SORTIE/libCoreMin.a" $OBJETS
vert "  ok    libCoreMin.a ($(du -h "$SORTIE/libCoreMin.a" | cut -f1))"

info "  CXX   temoin LibCore"
$CXX $CXXFLAGS "$RACINE/tools/ladybird/libcore-probe.cpp" -o "$SORTIE/libcore-probe" \
    "$SORTIE/libCoreMin.a" "$AK/libAK.a" \
    -L"$DEPS/lib" -lfmt -lsimdutf -lmimalloc -lpthread

vert "temoin construit : $SORTIE/libcore-probe"
if [ "$CIBLE" = "hote" ]; then
    echo
    "$SORTIE/libcore-probe"
fi
