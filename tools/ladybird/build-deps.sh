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
VER_SIMDJSON=4.6.4
VER_PSL=0.21.5

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

# Comme `recupere`, mais avec les sous-modules : libpsl porte la liste des
# suffixes publics dans un sous-module, et sans elle la table integree est vide.
recupere_recursif() {
    local depot=$1 tag=$2 dossier=$3
    if [ -d "$SRC/$dossier" ]; then
        return 0
    fi
    info "  FETCH $dossier ($tag, avec sous-modules)"
    git -c advice.detachedHead=false clone --quiet --depth 1 --recursive \
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

# --- simdjson ---------------------------------------------------------------
#
# LibJS s'en sert pour `JSON.parse`. Comme simdutf, il se distribue en amalgame :
# un `.cpp` et un `.h` uniques, deja produits dans l'arbre `singleheader/`.
recupere "https://github.com/simdjson/simdjson.git" \
    "v$VER_SIMDJSON" "simdjson-$VER_SIMDJSON"
if [ ! -f "$PREFIXE/lib/libsimdjson.a" ]; then
    info "  CXX   simdjson (peut prendre une minute)"
    ( cd "$SRC/simdjson-$VER_SIMDJSON/singleheader"
      $CXX $CXXSTD -c simdjson.cpp -o /tmp/simdjson.o
      ar rcs "$PREFIXE/lib/libsimdjson.a" /tmp/simdjson.o )
fi
cp "$SRC/simdjson-$VER_SIMDJSON/singleheader/simdjson.h" "$PREFIXE/include/"
vert "  ok    simdjson $VER_SIMDJSON"

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

# --- libpsl -----------------------------------------------------------------
#
# La liste publique des suffixes de domaine (`.com`, `.co.uk`, `.github.io`…),
# dont `LibURL/PublicSuffixData.cpp` se sert pour decider si deux hotes
# appartiennent au meme site. C'est une regle de securite, pas une commodite :
# elle empeche un cookie pose sur `.co.uk` de valoir pour tout le Royaume-Uni.
#
# `psl_builtin()` renvoie une table **compilee dans la bibliotheque** : aucun
# fichier n'est lu a l'execution, ce qui convient a Bouchaud ou rien ne garantit
# un chemin de donnees a cote du binaire. La table est produite ici par le
# script Python d'amont a partir de `list/public_suffix_list.dat`, un sous-module
# du depot — d'ou le clone `--recursive`.
recupere_recursif "https://github.com/rockdaboot/libpsl.git" \
    "$VER_PSL" "libpsl-$VER_PSL"
if [ ! -f "$PREFIXE/lib/libpsl.a" ]; then
    info "  GEN   suffixes publics"
    ( cd "$SRC/libpsl-$VER_PSL"
      # `include/libpsl.h` n'existe pas dans l'arbre : meson le produit depuis
      # `libpsl.h.in` en y substituant la version. Cinq jetons, aucun choix a
      # faire — on fait la meme substitution plutot que d'installer meson.
      if [ ! -f include/libpsl.h ]; then
          info "  GEN   libpsl.h"
          maj=${VER_PSL%%.*}; reste=${VER_PSL#*.}
          min=${reste%%.*}; corr=${reste#*.}
          sed -e "s|@LIBPSL_VERSION@|$VER_PSL|" \
              -e "s|@LIBPSL_VERSION_MAJOR@|$maj|" \
              -e "s|@LIBPSL_VERSION_MINOR@|$min|" \
              -e "s|@LIBPSL_VERSION_PATCH@|$corr|" \
              -e "s|@LIBPSL_VERSION_NUMBER@|$(printf '0x%02x%02x%02x' "$maj" "$min" "$corr")|" \
              include/libpsl.h.in > include/libpsl.h
      fi
      # `psl.c` inclut `suffixes_dafsa.h` — extension `.h`, et seulement sous
      # `ENABLE_BUILTIN`. Sans ce drapeau, il compile un `kDafsa[] = ""` vide,
      # `psl_builtin()` rend NULL, et — c'est la le danger —
      # `psl_is_public_suffix(NULL, ...)` repond **1 pour tout domaine**. Aucune
      # erreur nulle part, et la regle de meme-site s'effondre en silence : plus
      # aucun couple d'hotes n'appartiendrait au meme site.
      [ -f src/suffixes_dafsa.h ] || \
          python3 src/psl-make-dafsa --output-format=cxx+ \
              list/public_suffix_list.dat src/suffixes_dafsa.h
      info "  CC    libpsl"
      # libpsl passe normalement par meson, qui produit un `config.h`. On lui
      # donne ici la configuration minimale a la main.
      #
      # Le point qui merite d'etre retenu : les bibliotheques d'IDN se
      # selectionnent par `#ifdef WITH_LIBICU`, `#ifdef WITH_LIBIDN2`… donc
      # **ne pas les definir du tout**. Un `-DWITH_LIBICU=0` les activerait :
      # `#ifdef` teste la definition, pas la valeur. L'erreur ne se voit pas a
      # la compilation — elle sort a l'edition de liens, en references non
      # resolues vers `uidna_openUTS46_74`, c'est-a-dire vers l'ICU 74 du
      # systeme que le portage a justement ecarte.
      #
      # Aucune de ces bibliotheques n'est necessaire : `LibURL` normalise deja
      # les hotes avant d'appeler `psl_is_public_suffix2()`.
      #
      # `lookup_string_in_fixed_set.c` porte la recherche dans l'automate
      # produit plus haut ; sans lui, `LookupStringInFixedSet` reste non resolu.
      for c in psl lookup_string_in_fixed_set; do
          ${CC:-gcc} -O2 -DNDEBUG -fPIC $CXXFLAGS_SUP \
              -DHAVE_UNISTD_H=1 -DHAVE_STRNDUP=1 -DHAVE_CLOCK_GETTIME=1 \
              -DENABLE_BUILTIN=1 \
              -DPSL_FILE='"/dev/null"' -DPACKAGE_VERSION='"'"$VER_PSL"'"' \
              -Iinclude -Isrc -c "src/$c.c" -o "/tmp/psl-$c.o"
      done
      ar rcs "$PREFIXE/lib/libpsl.a" /tmp/psl-psl.o /tmp/psl-lookup_string_in_fixed_set.o )
fi
cp "$SRC/libpsl-$VER_PSL/include/libpsl.h" "$PREFIXE/include/"
vert "  ok    libpsl $VER_PSL"

echo
vert "dependances pretes dans third_party/deps-$CIBLE"
ls -1 "$PREFIXE/lib"
