#!/bin/bash
# Construit la caisse Rust propre a LibGfx au SHA Ladybird epingle.

set -eu
cd "$(dirname "$0")/../.."
RACINE=$(pwd)

LB="$RACINE/third_party/ladybird"
CIBLE=hote
[ "${1:-}" = "--cible" ] && CIBLE=bouchaud

TRIPLET=x86_64-unknown-linux-gnu
SORTIE="$RACINE/third_party/build-rust-gfx-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

[ -f "$LB/Libraries/LibGfx/Rust/Cargo.toml" ] || {
    rouge "Ladybird absent — lancer tools/ladybird/fetch.sh"
    exit 1
}

info "== libgfx_rust ($CIBLE) =="

# Comme les autres caisses du port, elle utilise le triplet Linux-gnu :
# Bouchaud expose cette ABI en ring 3. Le drapeau allocator reproduit
# import_rust_crate(... FEATURES allocator) de LibGfx/CMakeLists.txt.
(
    cd "$LB"
    CARGO_BUILD_TARGET="$TRIPLET" \
    CARGO_TARGET_DIR="$SORTIE" \
        cargo build --release --quiet -p libgfx_rust --features allocator
)

ARCHIVE="$SORTIE/$TRIPLET/release/liblibgfx_rust.a"
[ -f "$ARCHIVE" ] || { rouge "liblibgfx_rust.a absent"; exit 1; }

ENTETE=$(find "$SORTIE" -path '*libgfx_rust*' -name RustFFI.h -print -quit)
[ -n "$ENTETE" ] || { rouge "RustFFI.h de LibGfx introuvable"; exit 1; }

mkdir -p "$SORTIE/gen/LibGfx"
cp "$ENTETE" "$SORTIE/gen/LibGfx/RustFFI.h"

vert "libgfx_rust pret"
ls -lh "$ARCHIVE" | sed 's/^/  /'
