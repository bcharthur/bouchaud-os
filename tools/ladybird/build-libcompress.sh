#!/bin/bash
# Construit le vrai LibCompress upstream. LibGfx en depend directement.

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

AK="$RACINE/third_party/build-ak-$CIBLE"
GEN="$RACINE/third_party/gen-$CIBLE"
CORE="$RACINE/third_party/build-libcore-$CIBLE"
CRYPTO="$RACINE/third_party/build-libcrypto-$CIBLE"
ICU="$RACINE/third_party/icu-$CIBLE"
VCPKG="$RACINE/third_party/vcpkg-gfx-installed/x64-linux"
SORTIE="$RACINE/third_party/build-libcompress-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

for f in "$AK/libAK.a" "$CORE/libCore.a" "$CRYPTO/libCryptoMin.a" "$VCPKG/lib/libz.a"; do
    [ -f "$f" ] || { rouge "dependance absente : $f"; exit 1; }
done

mkdir -p "$SORTIE/obj"
CXX=${CXX:-clang++}
CXXFLAGS="-std=c++23 -O2 -fno-exceptions -fPIC \
    -DAK_SYSTEM_CACHE_ALIGNMENT_SIZE=64 $FLAGS_CIBLE \
    -I$LB -I$LB/Libraries -I$AK/gen -I$GEN \
    -I$VCPKG/include -I$ICU/include \
    -Wno-unused-parameter -Wno-unknown-pragmas -Wno-invalid-constexpr \
    -Wno-unqualified-std-cast-call -Wno-user-defined-literals \
    -Wno-unknown-warning-option"

SOURCES=$(sed -n '/^set(SOURCES/,/^)/p' "$LB/Libraries/LibCompress/CMakeLists.txt" \
          | grep -oE '[A-Za-z0-9_/]+\.cpp')

info "== LibCompress ($CIBLE) : $(echo "$SOURCES" | wc -w) fichiers =="

OBJETS=""
ECHECS=0
for src in $SOURCES; do
    nom=$(echo "${src%.cpp}" | tr / -)
    obj="$SORTIE/obj/$nom.o"
    OBJETS="$OBJETS $obj"
    if [ -f "$obj" ] && [ "$obj" -nt "$LB/Libraries/LibCompress/$src" ]; then
        continue
    fi
    if ! $CXX $CXXFLAGS -c "$LB/Libraries/LibCompress/$src" -o "$obj" \
        2> "${obj%.o}.log"; then
        rouge "ECHEC LibCompress/$src"
        grep -m4 "error:" "${obj%.o}.log" | sed 's/^/  /' || true
        ECHECS=$((ECHECS + 1))
    fi
done

[ "$ECHECS" -eq 0 ] || { rouge "$ECHECS source(s) en echec"; exit 1; }

ar rcs "$SORTIE/libCompress.a" $OBJETS
vert "libCompress.a pret ($(du -h "$SORTIE/libCompress.a" | cut -f1))"
