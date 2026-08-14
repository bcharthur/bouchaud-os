#!/bin/bash
# Construit AK, la brique de base de Ladybird, et joue un temoin.
#
#   ./tools/ladybird/build-ak.sh          hote
#   ./tools/ladybird/build-ak.sh --cible  Bouchaud (static-pie)
#
# ## Pourquoi une construction a nous plutot que le CMake d'upstream
#
# Le CMake de Ladybird construit *tout* Ladybird : il exige vcpkg, une centaine
# de paquets, Skia, Qt, ICU. Pour AK — qui ne depend que de quatre petites
# bibliotheques — c'est un cout sans rapport avec le besoin, et surtout un cout
# qu'il faudrait payer avant de savoir si AK compile pour notre cible.
#
# Ce script fait donc l'inverse : il lit la liste des sources dans
# `AK/CMakeLists.txt` d'upstream — pas une copie qui derive — et compile
# exactement ces fichiers. Quand upstream ajoute un fichier a AK, on le prend
# sans rien modifier ici.
#
# ## Les deux fichiers generes
#
# `AK/Debug.h` et `AK/Backtrace.h` n'existent pas dans l'arbre : CMake les
# produit depuis `.in`. On fait la meme chose, avec les valeurs qui conviennent
# a un port debutant (aucun drapeau de debogage, pas de trace de pile).

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
SORTIE="$RACINE/third_party/build-ak-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

[ -d "$LB/AK" ] || { rouge "Ladybird absent — lancer tools/ladybird/fetch.sh"; exit 1; }
[ -d "$DEPS/lib" ] || { rouge "dependances absentes — lancer tools/ladybird/build-deps.sh ${1:-}"; exit 1; }

CXX=${CXX:-clang++}
# `-fno-exceptions` : c'est ce qu'impose upstream (Meta/CMake/compile_options.cmake).
# `-DAK_DONT_REPLACE_STD` n'est pas pose : AK fournit ses propres `std::` minimaux
# et upstream compte dessus.
# `-Wno-invalid-constexpr` : `AK/Utf16String.h` declare des constructeurs
# `constexpr` dont le corps ne peut jamais etre evalue a la compilation. C'est
# **valide en C++23** — P2448R2 a precisement leve cette restriction — mais
# Clang 18 continue d'en faire une erreur. Upstream ne pose pas ce drapeau parce
# que sa chaine de reference est plus recente. C'est donc une accommodation de
# version cote Bouchaud, pas une divergence : aucune ligne de Ladybird n'est
# touchee, et le drapeau disparaitra avec la montee de Clang.
#
# `-Wno-unqualified-std-cast-call` : AK appelle `move()` sans qualification, par
# choix delibere (il fournit son propre `AK::move`). Des dizaines
# d'avertissements qui noieraient les vraies erreurs.
CXXFLAGS="-std=c++23 -O2 -fno-exceptions -fno-rtti -fPIC $FLAGS_CIBLE \
    -I$LB -I$SORTIE/gen -I$DEPS/include \
    -Wno-unused-parameter -Wno-unknown-pragmas \
    -Wno-invalid-constexpr -Wno-unqualified-std-cast-call \
    -Wno-user-defined-literals -Wno-unknown-warning-option"

mkdir -p "$SORTIE/obj" "$SORTIE/gen/AK"

# --- En-tetes generes -------------------------------------------------------
#
# `Debug.h.in` est une liste de `#cmakedefine XXX_DEBUG`. CMake les remplace par
# `#define` ou par un commentaire selon les options. Aucun drapeau de debogage
# n'etant demande ici, tous deviennent des `#define ... 0`.
# Les directives sont indentees (`#    cmakedefine01 X`) et enveloppees dans des
# `#ifndef` : on transforme donc le fichier **en place** plutot que d'en extraire
# des lignes. Un `#cmakedefine01 X` devient `#define X 0`, un `#cmakedefine X`
# devient un `#undef` commente — c'est exactement ce que fait `configure_file`
# quand l'option n'est pas posee.
genere_entete() {
    local entree=$1 sortie=$2
    if [ -f "$sortie" ] && [ "$sortie" -nt "$entree" ]; then
        return 0
    fi
    info "  GEN   $(basename "$sortie")"
    {
        echo "/* Genere par tools/ladybird/build-ak.sh depuis $(basename "$entree")."
        echo "   Aucun drapeau de debogage n'est pose : un port qui demarre veut"
        echo "   d'abord compiler, le bavardage vient apres. */"
        sed -E 's|^([[:space:]]*)#([[:space:]]*)cmakedefine01[[:space:]]+([A-Za-z0-9_]+)|\1#\2define \3 0|;
                s|^([[:space:]]*)#([[:space:]]*)cmakedefine[[:space:]]+([A-Za-z0-9_]+)|\1/* #undef \3 */|' \
            "$entree"
    } > "$sortie"
}

genere_entete "$LB/AK/Debug.h.in" "$SORTIE/gen/AK/Debug.h"

# Un port qui demarre n'a pas de trace de pile : c'est un confort de diagnostic,
# pas une dependance de correction.
genere_entete "$LB/AK/Backtrace.h.in" "$SORTIE/gen/AK/Backtrace.h"

# --- Liste des sources, lue chez upstream -----------------------------------
#
# On extrait le bloc `set(SOURCES ...)` de `AK/CMakeLists.txt`. C'est la seule
# facon de ne pas maintenir une seconde liste qui divergerait au premier fichier
# ajoute en amont.
SOURCES=$(sed -n '/^set(SOURCES/,/^)/p' "$LB/AK/CMakeLists.txt" \
          | grep -oE '[A-Za-z0-9_]+\.cpp')
# Sur une cible non Windows, upstream ajoute LexicalPath.cpp hors du bloc.
SOURCES="$SOURCES LexicalPath.cpp"

NB=$(echo "$SOURCES" | wc -w)
info "== AK ($CIBLE) : $NB fichiers =="

ECHECS=0
OBJETS=""
for src in $SOURCES; do
    obj="$SORTIE/obj/${src%.cpp}.o"
    OBJETS="$OBJETS $obj"
    if [ -f "$obj" ] && [ "$obj" -nt "$LB/AK/$src" ]; then
        continue
    fi
    if ! $CXX $CXXFLAGS -c "$LB/AK/$src" -o "$obj" 2> "$SORTIE/obj/${src%.cpp}.log"; then
        rouge "  ECHEC $src"
        # Les erreurs, pas les avertissements : AK en produit des dizaines
        # (`-Wunqualified-std-cast-call`) qui noieraient la cause reelle.
        grep -m3 "error:" "$SORTIE/obj/${src%.cpp}.log" | sed 's/^/          /'
        ECHECS=$((ECHECS + 1))
    fi
done

if [ "$ECHECS" -gt 0 ]; then
    rouge "$ECHECS fichier(s) d'AK n'ont pas compile"
    exit 1
fi

ar rcs "$SORTIE/libAK.a" $OBJETS
vert "  ok    libAK.a ($(du -h "$SORTIE/libAK.a" | cut -f1))"

# --- Temoin -----------------------------------------------------------------
info "  CXX   temoin AK"
$CXX $CXXFLAGS "$RACINE/tools/ladybird/ak-probe.cpp" -o "$SORTIE/ak-probe" \
    "$SORTIE/libAK.a" -L"$DEPS/lib" -lfmt -lsimdutf -lmimalloc -lpthread

vert "temoin construit : $SORTIE/ak-probe"
if [ "$CIBLE" = "hote" ]; then
    echo
    "$SORTIE/ak-probe"
fi
