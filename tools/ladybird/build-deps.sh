#!/bin/bash
# Construit les dependances tierces dont AK a besoin, aux versions epinglees.
#
#   ./tools/ladybird/build-deps.sh          pour l'hote
#   ./tools/ladybird/build-deps.sh --cible  pour Bouchaud (static-pie)
#
# ## Pourquoi pas vcpkg
#
# Upstream passe par vcpkg, qui suppose un systeme de paquets et une cible
# connue. Bouchaud n'est ni l'un ni l'autre : ce que nous produisons est une
# bibliotheque statique pour un ELF `-static-pie`. Ce script fait donc la meme
# chose que vcpkg, en petit, et surtout **aux memes versions** — celles de
# `vcpkg.json`, recopiees ici et nulle part ailleurs.
#
# ## Ce qu'AK reclame
#
#   fast_float   conversion chaine -> flottant (en-tetes seuls)
#   fmt          formatage
#   simdutf      validation et conversion UTF (amalgame : un .cpp, un .h)
#   mimalloc     allocateur ; `AK/kmalloc.cpp` l'appelle directement
#
# Aucune n'est optionnelle : `AK/CMakeLists.txt` les lie toutes les quatre.

set -eu
cd "$(dirname "$0")/../.."
RACINE=$(pwd)

# Versions : `third_party/ladybird/vcpkg.json`, section "overrides".
VER_FMT=12.2.0
VER_FASTFLOAT=8.2.10
VER_SIMDUTF=9.0.0
VER_MIMALLOC=2.2.7
VER_TOMMATH=1.3.0

CIBLE=hote
CXXFLAGS_SUP=""
if [ "${1:-}" = "--cible" ]; then
    CIBLE=bouchaud
    # Meme chaine que le navigateur : voir tools/userland/build-navigateur.sh.
    CXXFLAGS_SUP="-static-pie -fPIE"
fi

SRC="$RACINE/third_party/deps-src"
PREFIXE="$RACINE/third_party/deps-$CIBLE"
mkdir -p "$SRC" "$PREFIXE/include" "$PREFIXE/lib"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

CXX=${CXX:-clang++}
CXXSTD="-std=c++23 -O2 -fno-exceptions $CXXFLAGS_SUP"

# Recupere une source a un tag precis, une seule fois.
#
# `git clone --depth 1 --branch <tag>` plutot qu'une archive : les archives
# `codeload.github.com` ne traversent pas tous les mandataires d'entreprise,
# alors que le protocole git passe partout ou le depot Ladybird passe deja. Le
# tag fige la version aussi surement qu'une archive.
recupere() {
    local depot=$1 tag=$2 dossier=$3
    if [ -d "$SRC/$dossier" ]; then
        return 0
    fi
    info "  FETCH $dossier ($tag)"
    git -c advice.detachedHead=false clone --quiet --depth 1 \
        --branch "$tag" "$depot" "$SRC/$dossier"
}

info "== dependances AK ($CIBLE) =="

# --- fast_float : en-tetes seuls -------------------------------------------
recupere "https://github.com/fastfloat/fast_float.git" \
    "v$VER_FASTFLOAT" "fast_float-$VER_FASTFLOAT"
cp -r "$SRC/fast_float-$VER_FASTFLOAT/include/fast_float" "$PREFIXE/include/"
vert "  ok    fast_float $VER_FASTFLOAT (en-tetes)"

# --- fmt --------------------------------------------------------------------
recupere "https://github.com/fmtlib/fmt.git" \
    "$VER_FMT" "fmt-$VER_FMT"
if [ ! -f "$PREFIXE/lib/libfmt.a" ]; then
    info "  CXX   fmt"
    ( cd "$SRC/fmt-$VER_FMT"
      $CXX $CXXSTD -Iinclude -c src/format.cc -o /tmp/fmt-format.o
      ar rcs "$PREFIXE/lib/libfmt.a" /tmp/fmt-format.o )
fi
cp -r "$SRC/fmt-$VER_FMT/include/fmt" "$PREFIXE/include/"
vert "  ok    fmt $VER_FMT"

# --- simdutf ----------------------------------------------------------------
recupere "https://github.com/simdutf/simdutf.git" \
    "v$VER_SIMDUTF" "simdutf-$VER_SIMDUTF"
if [ ! -f "$PREFIXE/lib/libsimdutf.a" ]; then
    info "  CXX   simdutf (peut prendre une minute)"
    ( cd "$SRC/simdutf-$VER_SIMDUTF"
      # `-Isrc` en plus de `-Iinclude` : l'amalgame inclut ses propres
      # en-tetes par des chemins relatifs a `src/`.
      $CXX $CXXSTD -Iinclude -Isrc -c src/simdutf.cpp -o /tmp/simdutf.o
      ar rcs "$PREFIXE/lib/libsimdutf.a" /tmp/simdutf.o )
fi
cp -r "$SRC/simdutf-$VER_SIMDUTF/include/"* "$PREFIXE/include/"
vert "  ok    simdutf $VER_SIMDUTF"

# --- mimalloc ---------------------------------------------------------------
#
# `AK/kmalloc.cpp` appelle `mi_malloc`, `mi_heap_*` : ce n'est pas une
# preference d'allocateur, c'est une dependance de code. On le construit donc
# plutot que de diverger des le premier jour.
recupere "https://github.com/microsoft/mimalloc.git" \
    "v$VER_MIMALLOC" "mimalloc-$VER_MIMALLOC"
if [ ! -f "$PREFIXE/lib/libmimalloc.a" ]; then
    info "  CC    mimalloc"
    ( cd "$SRC/mimalloc-$VER_MIMALLOC"
      ${CC:-gcc} -O2 -DNDEBUG -DMI_MALLOC_OVERRIDE=0 $CXXFLAGS_SUP \
          -Iinclude -c src/static.c -o /tmp/mimalloc.o
      ar rcs "$PREFIXE/lib/libmimalloc.a" /tmp/mimalloc.o )
fi
cp -r "$SRC/mimalloc-$VER_MIMALLOC/include/"*.h "$PREFIXE/include/"
vert "  ok    mimalloc $VER_MIMALLOC"

# --- libtommath -------------------------------------------------------------
#
# L'arithmetique a precision arbitraire derriere `Crypto::UnsignedBigInteger`,
# donc derriere le type `BigInt` de JavaScript.
#
# La distribution en fournit **1.2.1**, et il a fallu le constater plutot que
# l'esperer : `UnsignedBigInteger.cpp` appelle `mp_expt_n()`, qui n'existe qu'a
# partir de **1.3.0** (c'est `mp_expt_d()` qui a ete renomme). Deux fichiers
# sur trois refusaient de compiler. L'API de libtommath n'est donc pas stable
# entre ces deux versions, contrairement a ce qu'on pouvait supposer — d'ou la
# construction depuis la version epinglee par `vcpkg.json`.
#
# `makefile.include` de libtommath impose ses propres `CFLAGS` ; on compile
# donc les sources directement, ce qui evite aussi d'avoir a lui expliquer
# `-static-pie`.
recupere "https://github.com/libtom/libtommath.git" \
    "v$VER_TOMMATH" "libtommath-$VER_TOMMATH"
if [ ! -f "$PREFIXE/lib/libtommath.a" ]; then
    info "  CC    libtommath"
    ( cd "$SRC/libtommath-$VER_TOMMATH"
      rm -rf /tmp/tommath-obj && mkdir -p /tmp/tommath-obj
      for c in *.c; do
          ${CC:-gcc} -O2 -DNDEBUG -fPIC $CXXFLAGS_SUP \
              -c "$c" -o "/tmp/tommath-obj/${c%.c}.o"
      done
      ar rcs "$PREFIXE/lib/libtommath.a" /tmp/tommath-obj/*.o )
fi
cp "$SRC/libtommath-$VER_TOMMATH/tommath.h" "$PREFIXE/include/"
vert "  ok    libtommath $VER_TOMMATH"

echo
vert "dependances pretes dans third_party/deps-$CIBLE"
ls -1 "$PREFIXE/lib"
