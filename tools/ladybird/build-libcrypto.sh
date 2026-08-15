#!/bin/bash
# Construit le sous-ensemble de LibCrypto dont LibJS a besoin.
#
#   ./tools/ladybird/build-libcrypto.sh          hote
#   ./tools/ladybird/build-libcrypto.sh --cible  Bouchaud (static-pie)
#
# ## Pourquoi un sous-ensemble, et pourquoi celui-ci
#
# LibCrypto complet, c'est 29 sources et deux dependances : `libtommath` et
# `OpenSSL::Crypto`. La mesure des inclusions dit que LibJS n'en emploie que
# **quatre en-tetes**, tous du meme cote :
#
#     11  LibCrypto/BigInt/SignedBigInteger.h
#      5  LibCrypto/BigFraction/BigFraction.h
#      4  LibCrypto/BigInt/UnsignedBigInteger.h
#      2  LibCrypto/Forward.h
#
# Autrement dit : LibJS ne veut pas de cryptographie. Il veut de l'arithmetique
# a precision arbitraire, pour le type `BigInt` du langage et pour les
# conversions numeriques exactes. Rien dans `Cipher/`, `Hash/`, `PK/`,
# `Certificate/` ou `ASN1/` n'est atteint depuis LibJS.
#
# ## Ce que cela evite, concretement
#
# Tout OpenSSL. Et ce n'est pas une economie de confort : la distribution
# fournit OpenSSL **3.0.13**, quand `vcpkg.json` d'upstream epingle **3.6.3**.
# `PK/MLKEM.cpp` et `PK/MLDSA.cpp` implementent ML-KEM et ML-DSA — les
# algorithmes post-quantiques — via `OSSL_PKEY_PARAM_ML_KEM_SEED` et consorts,
# qui n'existent qu'a partir d'OpenSSL 3.5. LibCrypto ne porte aucun garde de
# version : ces deux fichiers ne peuvent pas compiler contre 3.0.
#
# C'est exactement le probleme qu'ICU nous a deja pose, et il aurait coute une
# seconde construction depuis les sources. La mesure des inclusions montre
# qu'on n'a pas a le payer maintenant : OpenSSL redeviendra necessaire avec
# LibTLS et RequestServer, c'est-a-dire avec le reseau, pas avec le moteur
# JavaScript. La dette est notee dans docs/ladybird/LIBJS_PORT.md plutot que
# reglee d'avance.
#
# ## libtommath : la version compte aussi ici
#
# La distribution donne 1.2.1, `vcpkg.json` epingle 1.3.0, et l'ecart ne passe
# pas : `UnsignedBigInteger.cpp` appelle `mp_expt_n()`, qui n'existe qu'a partir
# de 1.3.0 — c'est `mp_expt_d()` qui y a ete renomme. Deux des trois sources
# refusent de compiler contre 1.2.1.
#
# C'est la troisieme fois de ce portage que la version d'une bibliotheque
# tierce, et non le code, decide de ce qui compile — apres ICU 74 contre 77, et
# OpenSSL 3.0 contre 3.5. La regle qui s'en degage : on construit ce que
# `vcpkg.json` epingle, et on ne prend une version de la distribution qu'apres
# l'avoir vue passer. `build-deps.sh` construit donc libtommath 1.3.0.

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
CORE="$RACINE/third_party/build-libcore-$CIBLE"
SORTIE="$RACINE/third_party/build-libcrypto-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

[ -f "$CORE/libCore.a" ] || { rouge "LibCore absent — lancer build-libcore.sh ${1:-}"; exit 1; }
[ -f "$DEPS/lib/libtommath.a" ] || {
    rouge "libtommath absent — lancer build-deps.sh ${1:-}"
    rouge "  (celui de la distribution est trop vieux : il n'a pas mp_expt_n)"
    exit 1
}

info "== LibCrypto, sous-ensemble ($CIBLE) =="

CXX=${CXX:-clang++}
# `-DAK_SYSTEM_CACHE_ALIGNMENT_SIZE=64` : voir build-ak.sh.
CXXFLAGS="-std=c++23 -O2 -fno-exceptions -fPIC -DAK_SYSTEM_CACHE_ALIGNMENT_SIZE=64 $FLAGS_CIBLE \
    -I$LB -I$LB/Libraries -I$AK/gen -I$GEN -I$DEPS/include \
    -Wno-unused-parameter -Wno-unknown-pragmas -Wno-invalid-constexpr \
    -Wno-unqualified-std-cast-call -Wno-user-defined-literals \
    -Wno-unknown-warning-option"

mkdir -p "$SORTIE/obj"

# Le sous-ensemble reclame par LibJS, et rien d'autre. Si l'editeur de liens de
# LibJS reclame un symbole de plus, c'est ici qu'on ajoute le fichier qui le
# porte — comme pour LibCore, la liste vient de la mesure, pas d'une intuition.
SOURCES="BigInt/UnsignedBigInteger.cpp BigInt/SignedBigInteger.cpp \
    BigFraction/BigFraction.cpp"

ECHECS=0
OBJETS=""
for src in $SOURCES; do
    obj="$SORTIE/obj/$(echo "${src%.cpp}" | tr / -).o"
    OBJETS="$OBJETS $obj"
    if [ -f "$obj" ] && [ "$obj" -nt "$LB/Libraries/LibCrypto/$src" ]; then
        continue
    fi
    if ! $CXX $CXXFLAGS -c "$LB/Libraries/LibCrypto/$src" -o "$obj" 2> "${obj%.o}.log"; then
        rouge "  ECHEC $src"
        grep -m3 "error:" "${obj%.o}.log" | sed 's/^/          /'
        ECHECS=$((ECHECS + 1))
    fi
done

if [ "$ECHECS" -gt 0 ]; then
    rouge "$ECHECS fichier(s) de LibCrypto n'ont pas compile"
    exit 1
fi

ar rcs "$SORTIE/libCryptoMin.a" $OBJETS
vert "  ok    libCryptoMin.a ($(du -h "$SORTIE/libCryptoMin.a" | cut -f1))"

info "  CXX   temoin LibCrypto"
$CXX $CXXFLAGS "$RACINE/tools/ladybird/libcrypto-probe.cpp" \
    -o "$SORTIE/libcrypto-probe" \
    "$SORTIE/libCryptoMin.a" "$CORE/libCore.a" "$AK/libAK.a" \
    -L"$DEPS/lib" -ltommath -lfmt -lsimdutf -lmimalloc -lpthread

vert "temoin construit : $SORTIE/libcrypto-probe"
if [ "$CIBLE" = "hote" ]; then
    echo
    "$SORTIE/libcrypto-probe"
fi
