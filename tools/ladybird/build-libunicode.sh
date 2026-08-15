#!/bin/bash
# Construit LibUnicode : ICU, la caisse Rust, et les 23 sources C++.
#
#   ./tools/ladybird/build-libunicode.sh          hote
#
# ## Pourquoi c'est le morceau qui coute
#
# `LibJS` lie `LibUnicode` en **public** — le commentaire d'upstream le dit :
# « Link LibUnicode publicly to ensure ICU data is available in any process using
# LibJS. » Il n'existe donc pas de LibJS sans LibUnicode, ni de LibUnicode sans
# ICU. C'est la seule dependance du chemin critique qu'on ne peut pas contourner.
#
# LibUnicode reclame en outre une **caisse Rust** (`libunicode_rust`) qui apporte
# les calendriers, et dont `cbindgen` genere l'en-tete FFI. Ladybird est donc un
# projet a deux chaines de compilation, et le portage Bouchaud aussi.
#
# ## Le piege du `.cargo/config.toml`
#
# Bouchaud epingle sa cible noyau (`x86_64-bouchaud_os.json`) dans
# `.cargo/config.toml` a la racine. Toute invocation de cargo faite depuis
# l'arbre en herite — y compris celle-ci, qui veut l'hote. D'ou le
# `CARGO_BUILD_TARGET` explicite : sans lui, cargo essaie de construire la caisse
# Unicode pour le noyau et echoue sur un message qui ne parle pas d'Unicode.

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
SORTIE="$RACINE/third_party/build-libunicode-$CIBLE"
RUST_OUT="$RACINE/third_party/build-rust-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

[ -f "$CORE/libCoreMin.a" ] || { rouge "LibCore absent — lancer build-libcore.sh"; exit 1; }

# --- ICU : le notre, pas celui de la distribution ---------------------------
#
# `libicu-dev` de Debian donne ICU 74, contre lequel 19 des 23 sources
# compilent — et les 4 dernieres pas du tout. Voir `build-icu.sh` pour le
# detail des API en cause. On exige donc l'ICU construit par ce script, et on
# refuse de retomber silencieusement sur celui du systeme : un repli
# silencieux ferait reapparaitre les memes 4 echecs, un mois plus tard, sans
# que rien ne rappelle pourquoi.
ICU="$RACINE/third_party/icu-$CIBLE"
[ -f "$ICU/lib/libicuuc.a" ] || {
    rouge "ICU absent — lancer tools/ladybird/build-icu.sh ${1:-}"
    exit 1
}
export PKG_CONFIG_PATH="$ICU/lib/pkgconfig"
ICU_VER=$(pkg-config --modversion icu-uc)
info "== LibUnicode ($CIBLE) — ICU $ICU_VER =="
# `vcpkg.json` d'upstream epingle 78.3, mais aucun tag `release-78-*` n'existe
# publiquement : `release-77-1` est le plus recent. L'ecart est assume et
# documente dans docs/ladybird/LIBJS_PORT.md.
case "$ICU_VER" in
    77.*) ;;
    *) info "  note  attendu 77.x (upstream ecrit 78.3, tag inexistant en amont)" ;;
esac

# --- La caisse Rust ---------------------------------------------------------
#
# Construite par `build-rust.sh`, avec les quatre autres caisses du chemin
# LibJS. Elle l'etait ici auparavant ; le regroupement n'est pas cosmetique :
# `libunicode_rust` est aussi un `rlib` consomme par `libregex_rust`, et la
# fonctionnalite `allocator` doit etre posee sur l'une sans l'etre sur l'autre.
# C'est une propriete du graphe entier, pas de cette bibliotheque seule.
RUST_LIB="$RUST_OUT/x86_64-unknown-linux-gnu/release/liblibunicode_rust.a"
[ -f "$RUST_LIB" ] || { rouge "caisses Rust absentes — lancer build-rust.sh ${1:-}"; exit 1; }
[ -f "$RUST_OUT/gen/LibUnicode/RustFFI.h" ] || { rouge "RustFFI.h absent — relancer build-rust.sh"; exit 1; }
vert "  ok    libunicode_rust + RustFFI.h"

mkdir -p "$SORTIE/obj" "$SORTIE/gen/LibUnicode"
cp "$RUST_OUT/gen/LibUnicode/RustFFI.h" "$SORTIE/gen/LibUnicode/RustFFI.h"

if [ ! -f "$SORTIE/gen/LibUnicode/Export.h" ]; then
    cat > "$SORTIE/gen/LibUnicode/Export.h" <<'EXPORT'
/* Genere par build-libunicode.sh : archive statique, macros vides. */
#pragma once
#define UNICODE_API
#define UNICODE_NO_EXPORT
EXPORT
fi

CXX=${CXX:-clang++}
# `-fno-rtti` n'est **pas** pose : upstream ne desactive que les exceptions
# (Meta/CMake/compile_options.cmake). `AK/TypeCasts.h` s'appuie sur
# `dynamic_cast`, et melanger des unites compilees avec et sans RTTI produit des
# transtypages qui echouent a l'execution sans rien dire.
CXXFLAGS="-std=c++23 -O2 -fno-exceptions -fPIC $FLAGS_CIBLE \
    -I$LB -I$LB/Libraries -I$AK/gen -I$CORE/gen -I$SORTIE/gen -I$DEPS/include \
    $(pkg-config --cflags icu-i18n icu-uc) \
    -Wno-unused-parameter -Wno-unknown-pragmas -Wno-invalid-constexpr \
    -Wno-unqualified-std-cast-call -Wno-user-defined-literals \
    -Wno-unknown-warning-option"

SOURCES=$(sed -n '/^set(SOURCES/,/^)/p' "$LB/Libraries/LibUnicode/CMakeLists.txt" \
          | grep -oE '[A-Za-z0-9_]+\.cpp')
NB=$(echo "$SOURCES" | wc -w)
info "  $NB fichiers C++"

ECHECS=0
OBJETS=""
for src in $SOURCES; do
    obj="$SORTIE/obj/${src%.cpp}.o"
    OBJETS="$OBJETS $obj"
    # Certaines sources vivent dans un sous-repertoire (`Calendars/`) que le
    # bloc `set(SOURCES ...)` mentionne avec son chemin : on les retrouve plutot
    # que de supposer qu'elles sont a la racine de la bibliotheque.
    chemin=$(find "$LB/Libraries/LibUnicode" -name "$src" -print -quit)
    [ -n "$chemin" ] || { rouge "  ECHEC $src introuvable"; ECHECS=$((ECHECS+1)); continue; }
    [ -f "$obj" ] && [ "$obj" -nt "$chemin" ] && continue
    if ! $CXX $CXXFLAGS -c "$chemin" -o "$obj" \
         2> "$SORTIE/obj/${src%.cpp}.log"; then
        rouge "  ECHEC $src"
        grep -m2 "error:" "$SORTIE/obj/${src%.cpp}.log" | sed 's/^/          /'
        ECHECS=$((ECHECS + 1))
    fi
done

if [ "$ECHECS" -gt 0 ]; then
    rouge "$ECHECS fichier(s) de LibUnicode n'ont pas compile"
    exit 1
fi

ar rcs "$SORTIE/libUnicode.a" $OBJETS
vert "  ok    libUnicode.a ($(du -h "$SORTIE/libUnicode.a" | cut -f1))"
echo "$RUST_LIB" > "$SORTIE/rust-lib.txt"

# --- Le temoin --------------------------------------------------------------
#
# `-licudata` **apres** `-licuuc` et `-licui18n` : l'archive de donnees ne
# reference rien, elle est referencee. L'ordre inverse la ferait ecarter par
# l'editeur de liens, et les donnees manqueraient a l'execution sans qu'aucune
# erreur de liaison ne le signale.
info "  CXX   temoin LibUnicode"
$CXX $CXXFLAGS "$RACINE/tools/ladybird/libunicode-probe.cpp" \
    -o "$SORTIE/libunicode-probe" \
    "$SORTIE/libUnicode.a" "$CORE/libCoreMin.a" "$AK/libAK.a" "$RUST_LIB" \
    $(pkg-config --libs icu-i18n icu-uc) \
    -L"$DEPS/lib" -lfmt -lsimdutf -lmimalloc -lpthread

vert "temoin construit : $SORTIE/libunicode-probe ($(du -h "$SORTIE/libunicode-probe" | cut -f1))"
if [ "$CIBLE" = "hote" ]; then
    echo
    "$SORTIE/libunicode-probe"
fi
