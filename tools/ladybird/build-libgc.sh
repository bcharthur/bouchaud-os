#!/bin/bash
# Construit LibGC, le ramasse-miettes de Ladybird, et joue un temoin.
#
#   ./tools/ladybird/build-libgc.sh          hote
#   ./tools/ladybird/build-libgc.sh --cible  Bouchaud (static-pie)
#
# ## Ce que ce jalon prouve
#
# LibGC est le premier composant de Ladybird dont la **correction depend du
# systeme** et pas seulement de la compilation. Un ramasse-miettes conservateur
# balaie les piles et les registres a la recherche de pointeurs ; s'il se trompe
# de zone, il ne plante pas — il recolte des objets vivants, et le defaut se
# manifeste ailleurs, plus tard, sans lien apparent avec sa cause.
#
# C'est pour cela que le temoin d'AK verifiait deja `StackInfo`, et que celui-ci
# force de vraies collectes plutot que de se contenter d'allouer.
#
# ## Ce qu'il ne demande pas
#
# ICU. LibGC ne reclame que six en-tetes de LibCore, LibSync et LibThreading —
# mesure faite sur ses inclusions, pas supposee. Le gros du tiers-parti n'arrive
# qu'avec LibJS.

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
CORE="$RACINE/third_party/build-libcore-$CIBLE"
SORTIE="$RACINE/third_party/build-libgc-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

[ -f "$CORE/libCoreMin.a" ] || { rouge "LibCore absent — lancer build-libcore.sh ${1:-}"; exit 1; }

CXX=${CXX:-clang++}
CXXFLAGS="-std=c++23 -O2 -fno-exceptions -fno-rtti -fPIC $FLAGS_CIBLE \
    -I$LB -I$LB/Libraries -I$AK/gen -I$CORE/gen -I$SORTIE/gen -I$DEPS/include \
    -Wno-unused-parameter -Wno-unknown-pragmas -Wno-invalid-constexpr \
    -Wno-unqualified-std-cast-call -Wno-user-defined-literals \
    -Wno-unknown-warning-option"

mkdir -p "$SORTIE/obj" "$SORTIE/gen/LibGC"

if [ ! -f "$SORTIE/gen/LibGC/Export.h" ]; then
    info "  GEN   LibGC/Export.h"
    cat > "$SORTIE/gen/LibGC/Export.h" <<'EXPORT'
/* Genere par tools/ladybird/build-libgc.sh : archive statique, donc macros
 * de visibilite vides. Voir build-libcore.sh pour l'explication complete. */
#pragma once
#define GC_API
#define GC_NO_EXPORT
EXPORT
fi

SOURCES=$(sed -n '/^set(SOURCES/,/^)/p' "$LB/Libraries/LibGC/CMakeLists.txt" \
          | grep -oE '[A-Za-z0-9_]+\.cpp')
NB=$(echo "$SOURCES" | wc -w)
info "== LibGC ($CIBLE) : $NB fichiers =="

ECHECS=0
OBJETS=""
for src in $SOURCES; do
    obj="$SORTIE/obj/${src%.cpp}.o"
    OBJETS="$OBJETS $obj"
    if [ -f "$obj" ] && [ "$obj" -nt "$LB/Libraries/LibGC/$src" ]; then
        continue
    fi
    if ! $CXX $CXXFLAGS -c "$LB/Libraries/LibGC/$src" -o "$obj" 2> "$SORTIE/obj/${src%.cpp}.log"; then
        rouge "  ECHEC $src"
        grep -m3 "error:" "$SORTIE/obj/${src%.cpp}.log" | sed 's/^/          /'
        ECHECS=$((ECHECS + 1))
    fi
done

if [ "$ECHECS" -gt 0 ]; then
    rouge "$ECHECS fichier(s) de LibGC n'ont pas compile"
    exit 1
fi

ar rcs "$SORTIE/libGC.a" $OBJETS
vert "  ok    libGC.a ($(du -h "$SORTIE/libGC.a" | cut -f1))"

info "  CXX   temoin LibGC"
$CXX $CXXFLAGS "$RACINE/tools/ladybird/libgc-probe.cpp" -o "$SORTIE/libgc-probe" \
    "$SORTIE/libGC.a" "$CORE/libCoreMin.a" "$AK/libAK.a" \
    -L"$DEPS/lib" -lfmt -lsimdutf -lmimalloc -lpthread

vert "temoin construit : $SORTIE/libgc-probe"
if [ "$CIBLE" = "hote" ]; then
    echo
    "$SORTIE/libgc-probe"
fi
