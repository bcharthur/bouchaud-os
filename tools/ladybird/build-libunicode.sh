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
pkg-config --exists icu-i18n icu-uc || { rouge "ICU absent — apt install libicu-dev"; exit 1; }

ICU_VER=$(pkg-config --modversion icu-i18n)
info "== LibUnicode ($CIBLE) — ICU $ICU_VER =="
# Upstream epingle ICU 78.3 dans vcpkg.json. Une version differente n'est pas
# forcement un probleme — l'API C++ d'ICU est stable — mais c'est un ecart a
# connaitre : si une fonction manque, c'est la premiere piste.
if [ "$ICU_VER" != "78.3" ]; then
    info "  note  upstream epingle ICU 78.3 ; ecart a garder en tete"
fi

# --- La caisse Rust ---------------------------------------------------------
CRATE="$LB/Libraries/LibUnicode/Rust"
if [ ! -f "$RUST_OUT/x86_64-unknown-linux-gnu/release/liblibunicode_rust.a" ]; then
    info "  CARGO libunicode_rust"
    ( cd "$CRATE"
      CARGO_BUILD_TARGET=x86_64-unknown-linux-gnu \
      CARGO_TARGET_DIR="$RUST_OUT" \
          cargo build --release --features allocator )
fi
RUST_LIB="$RUST_OUT/x86_64-unknown-linux-gnu/release/liblibunicode_rust.a"
RUST_FFI=$(find "$RUST_OUT" -name RustFFI.h -print -quit)
[ -n "$RUST_FFI" ] || { rouge "RustFFI.h introuvable (cbindgen n'a pas tourne ?)"; exit 1; }
vert "  ok    libunicode_rust + $(basename "$RUST_FFI")"

mkdir -p "$SORTIE/obj" "$SORTIE/gen/LibUnicode"
cp "$RUST_FFI" "$SORTIE/gen/LibUnicode/RustFFI.h"

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
