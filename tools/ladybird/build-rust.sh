#!/bin/bash
# Construit les caisses Rust de Ladybird dont le chemin LibJS a besoin.
#
#   ./tools/ladybird/build-rust.sh          hote
#   ./tools/ladybird/build-rust.sh --cible  Bouchaud
#
# ## Ladybird est un projet a deux chaines de compilation
#
# C'est le fait le plus structurant du portage, et celui qu'on decouvre le plus
# tard si on ne le cherche pas. `Cargo.toml` a la racine declare un espace de
# travail de dix caisses, et cinq d'entre elles sont sur le chemin de LibJS :
#
#     libunicode_rust    calendriers, indispensable a LibUnicode
#     libregex_rust      moteur d'expressions regulieres
#     libtextcodec_rust  decodage des encodages historiques
#     liburl_rust        analyse d'URL selon la norme WHATWG
#     libjs_rust         parties de LibJS elles-memes ecrites en Rust
#
# Chacune produit une archive statique **et** un en-tete FFI genere par
# `cbindgen` depuis son `build.rs`. Les deux comptent : sans l'en-tete, le C++
# ne compile pas ; sans l'archive, il ne se lie pas.
#
# Bouchaud sait deja construire du Rust — son noyau en est fait. Ce n'est donc
# pas une chaine a introduire, mais une chaine a ne pas laisser se melanger
# avec celle du noyau, ce qui est exactement le piege suivant.
#
# ## Le piege du `.cargo/config.toml`
#
# Bouchaud epingle sa cible noyau (`x86_64-bouchaud_os.json`) dans
# `.cargo/config.toml` a la racine du depot. **Toute** invocation de cargo faite
# depuis l'arbre en herite, y compris celles-ci, qui veulent l'hote. Sans
# `CARGO_BUILD_TARGET` explicite, cargo essaie de construire les caisses de
# Ladybird pour le noyau Bouchaud et echoue sur un message qui ne parle ni de
# Ladybird ni de cible — on cherche alors le defaut dans la mauvaise direction.
#
# ## Pourquoi une seule construction pour l'hote et la cible
#
# Meme raison que pour ICU (voir `build-icu.sh`) : la cible Bouchaud emploie
# aujourd'hui le meme triplet que l'hote, et ne s'en distingue qu'au moment de
# lier (`-static-pie`). Les archives sont donc les memes. Le jour ou Bouchaud
# aura son propre triplet Rust, c'est ici qu'il faudra le declarer — et le lien
# symbolique rend ce moment visible plutot que silencieux.

set -eu
cd "$(dirname "$0")/../.."
RACINE=$(pwd)

LB="$RACINE/third_party/ladybird"
CIBLE=hote
if [ "${1:-}" = "--cible" ]; then
    CIBLE=bouchaud
fi

TRIPLET=x86_64-unknown-linux-gnu
SORTIE="$RACINE/third_party/build-rust-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

[ -d "$LB/Libraries" ] || { rouge "Ladybird absent — lancer tools/ladybird/fetch.sh"; exit 1; }

# Les caisses du chemin LibJS : `caisse:bibliotheque C++:drapeaux de fonction`.
#
# La colonne des fonctionnalites n'est pas un choix : elle recopie exactement
# les appels `import_rust_crate(... FEATURES ...)` des `CMakeLists.txt`
# d'upstream. `libtextcodec_rust` et `libjs_rust` n'ont volontairement pas
# `allocator` ; voir plus bas pourquoi cela compte.
CAISSES="libunicode_rust:LibUnicode:allocator \
         libregex_rust:LibRegex:allocator \
         libtextcodec_rust:LibTextCodec: \
         liburl_rust:LibURL:allocator \
         libjs_rust:LibJS:"

if [ "$CIBLE" = bouchaud ] && [ -d "$RACINE/third_party/build-rust-hote/$TRIPLET" ]; then
    info "== caisses Rust (cible) : meme triplet que l'hote, partagees =="
    ln -sfn "$RACINE/third_party/build-rust-hote" "$SORTIE"
    vert "  ok    $SORTIE -> build-rust-hote"
    exit 0
fi

info "== caisses Rust ($CIBLE) — $TRIPLET =="

# ## Une invocation de cargo par caisse, et non une seule pour toutes
#
# La version courte : `cargo build -p a -p b --features allocator` ne compile
# pas, et la raison merite d'etre notee parce qu'elle n'a rien d'evident.
#
# `--features allocator` fait inclure `Libraries/RustAllocator.rs`, qui declare
# un `#[global_allocator]` renvoyant vers `ladybird_rust_alloc` — c'est ainsi
# que Rust et C++ partagent le tas d'AK (`AK/kmalloc.cpp`) au lieu d'en tenir
# deux. Il ne peut y en avoir qu'un par unite de liaison.
#
# Or `libunicode_rust` est declare `crate-type = ["staticlib", "rlib"]` : il est
# a la fois bibliotheque finale **et** dependance d'autres caisses, dont
# `libregex_rust`. Dans une invocation unique, cargo unifie les fonctionnalites
# sur tout le graphe : l'`allocator` demande pour la bibliotheque finale se
# retrouve aussi dans le rlib consomme par `libregex_rust`, qui en declare deja
# un. D'ou « the `#[global_allocator]` in this crate conflicts with global
# allocator in: libunicode_rust ».
#
# Une invocation par caisse rend a cargo des graphes distincts : chaque
# dependance est alors construite avec ses fonctionnalites par defaut, sans
# allocateur, et un seul `#[global_allocator]` subsiste par archive. C'est
# exactement ce que fait le CMake d'upstream, un `import_rust_crate` a la fois —
# ce qui n'etait pas un detail d'organisation, mais une condition de
# correction.
for entree in $CAISSES; do
    caisse=$(echo "$entree" | cut -d: -f1)
    biblio=$(echo "$entree" | cut -d: -f2)
    fonctions=$(echo "$entree" | cut -d: -f3)

    DRAPEAUX=""
    [ -n "$fonctions" ] && DRAPEAUX="--features $fonctions"

    info "  CARGO $caisse ${fonctions:+($fonctions)}"
    ( cd "$LB"
      CARGO_BUILD_TARGET="$TRIPLET" \
      CARGO_TARGET_DIR="$SORTIE" \
          cargo build --release --quiet -p "$caisse" $DRAPEAUX )
done

echo
LIBDIR="$SORTIE/$TRIPLET/release"
MANQUE=0
for entree in $CAISSES; do
    caisse=$(echo "$entree" | cut -d: -f1)
    biblio=$(echo "$entree" | cut -d: -f2)
    archive="$LIBDIR/lib$caisse.a"
    if [ ! -f "$archive" ]; then
        rouge "  ECHEC $caisse : archive absente"
        MANQUE=$((MANQUE + 1))
        continue
    fi

    # `cbindgen` ecrit `RustFFI.h` dans le repertoire `OUT_DIR` de la caisse,
    # dont le nom porte une empreinte que rien ne permet de predire. On le
    # retrouve donc, et on le range sous un nom stable que les scripts C++
    # peuvent inclure — `-I$SORTIE/gen`, puis `<LibXxx/RustFFI.h>`.
    entete=$(find "$SORTIE" -path "*$caisse*" -name RustFFI.h -print -quit)
    if [ -z "$entete" ]; then
        rouge "  ECHEC $caisse : RustFFI.h introuvable (cbindgen n'a pas tourne ?)"
        MANQUE=$((MANQUE + 1))
        continue
    fi
    mkdir -p "$SORTIE/gen/$biblio"
    cp "$entete" "$SORTIE/gen/$biblio/RustFFI.h"
    vert "  ok    $(printf '%-18s' "$caisse") $(du -h "$archive" | cut -f1)  ->  gen/$biblio/RustFFI.h"
done

[ "$MANQUE" -eq 0 ] || { rouge "$MANQUE caisse(s) manquante(s)"; exit 1; }
echo
vert "caisses Rust pretes dans $SORTIE"
