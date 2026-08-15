#!/bin/bash
# Produit les en-tetes que le CMake d'upstream genere, et qui n'existent pas
# dans l'arbre source.
#
#   ./tools/ladybird/build-entetes.sh          hote
#   ./tools/ladybird/build-entetes.sh --cible  Bouchaud
#
# Sortie : `third_party/gen-<cible>/`, a passer en `-I` a toutes les
# compilations Ladybird.
#
# ## Pourquoi un script a part
#
# Parce que ces en-tetes sont partages. `LibCore` inclut `LibURL/Export.h`,
# `LibJS` inclut `LibUnicode/RustFFI.h` : les faire produire par le script de
# chaque bibliotheque obligeait chacun a passer les `-I` des autres, et l'ordre
# de construction devenait un ordre de generation. Un seul arbre genere supprime
# la question.
#
# ## Export.h : seulement pour celles qui le declarent
#
# `generate_export_header` n'est appele que par `ladybird_lib(...
# EXPLICIT_SYMBOL_EXPORT)`. La liste est donc **lue** dans les `CMakeLists.txt`
# d'upstream, pas devinee — et le nom de la macro s'en deduit :
# `ladybird_lib(LibJS js EXPLICIT_SYMBOL_EXPORT)` donne `JS_API`, parce que
# `Meta/CMake/targets.cmake` fait `string(TOUPPER ${fs_name})` puis `_API`.
#
# Une version anterieure de ces scripts generait aussi `LibURL/Export.h`,
# `LibUnicode/Export.h`, `LibThreading/Export.h`… Aucune de ces bibliotheques
# ne declare `EXPLICIT_SYMBOL_EXPORT`, aucune n'inclut un `Export.h`, et les
# macros correspondantes (`URL_API`, `UNICODE_API`…) n'apparaissent nulle part
# dans Ladybird : c'etaient cinq fichiers inventes. Ils ne cassaient rien, mais
# ils faisaient diverger notre construction de celle d'upstream sans raison, et
# c'est exactement ce qu'on ne veut pas accumuler.
#
# ## Pourquoi les macros sont vides
#
# `generate_export_header` pilote la visibilite des symboles d'une bibliotheque
# **partagee**. Nous produisons des archives statiques : la visibilite ne se pose
# pas. C'est ce que CMake genere lui-meme quand `<NOM>_STATIC_DEFINE` est defini.

set -eu
cd "$(dirname "$0")/../.."
RACINE=$(pwd)

LB="$RACINE/third_party/ladybird"
CIBLE=hote
if [ "${1:-}" = "--cible" ]; then
    CIBLE=bouchaud
fi

RUST="$RACINE/third_party/build-rust-$CIBLE"
SORTIE="$RACINE/third_party/gen-$CIBLE"

rouge() { printf '\033[31m%s\033[0m\n' "$*"; }
vert()  { printf '\033[32m%s\033[0m\n' "$*"; }
info()  { printf '\033[36m%s\033[0m\n' "$*"; }

[ -d "$LB/Libraries" ] || { rouge "Ladybird absent — lancer tools/ladybird/fetch.sh"; exit 1; }

info "== en-tetes generes ($CIBLE) =="
mkdir -p "$SORTIE"

# --- Export.h ---------------------------------------------------------------
NB=0
for cmake in "$LB"/Libraries/*/CMakeLists.txt; do
    ligne=$(grep -oE 'ladybird_lib\([A-Za-z]+ [a-z0-9]+ EXPLICIT_SYMBOL_EXPORT\)' "$cmake" || true)
    [ -n "$ligne" ] || continue
    biblio=$(echo "$ligne" | sed -E 's/ladybird_lib\(([A-Za-z]+) .*/\1/')
    nom=$(echo "$ligne" | sed -E 's/ladybird_lib\([A-Za-z]+ ([a-z0-9]+) .*/\1/')
    macro=$(echo "$nom" | tr '[:lower:]' '[:upper:]')_API

    mkdir -p "$SORTIE/$biblio"
    cat > "$SORTIE/$biblio/Export.h" <<EXPORT
/* Genere par tools/ladybird/build-entetes.sh.
 *
 * Equivalent de ce que \`generate_export_header\` produit pour une bibliotheque
 * **statique** : les macros de visibilite sont vides, parce qu'une archive
 * n'exporte rien — tout est resolu a l'edition de liens finale.
 *
 * Le nom de la macro vient de \`ladybird_lib($biblio $nom EXPLICIT_SYMBOL_EXPORT)\`. */
#pragma once
#define ${macro}
#define ${macro%_API}_NO_EXPORT
EXPORT
    NB=$((NB + 1))
done
vert "  ok    $NB Export.h (lus dans les CMakeLists d'upstream)"

# --- RustFFI.h --------------------------------------------------------------
#
# Deux dispositions, parce que les bibliotheques ne s'accordent pas :
#
#     <RustFFI.h>             LibRegex, LibTextCodec
#     <LibJS/RustFFI.h>       LibJS
#     <LibURL/RustFFI.h>      LibURL
#     <LibUnicode/RustFFI.h>  LibUnicode
#
# La forme prefixee se resout par `-I$SORTIE`. La forme nue exige un `-I` par
# bibliotheque, sans quoi cinq fichiers homonymes se disputeraient le meme nom —
# et chacune recevrait l'interface d'une autre, sans erreur d'inclusion, juste
# des declarations qui ne correspondent a rien de ce qui sera lie. D'ou
# `$SORTIE/ffi/<biblio>/`, que les scripts ajoutent bibliotheque par
# bibliotheque. C'est ce que fait `FFI_OUTPUT_DIR` chez upstream, cible par
# cible.
if [ -d "$RUST/gen" ]; then
    NB=0
    for chemin in "$RUST"/gen/*/RustFFI.h; do
        [ -e "$chemin" ] || continue
        biblio=$(basename "$(dirname "$chemin")")
        mkdir -p "$SORTIE/$biblio" "$SORTIE/ffi/$biblio"
        cp "$chemin" "$SORTIE/$biblio/RustFFI.h"
        cp "$chemin" "$SORTIE/ffi/$biblio/RustFFI.h"
        NB=$((NB + 1))
    done
    vert "  ok    $NB RustFFI.h (prefixes et nus)"
else
    info "  note  caisses Rust absentes — lancer build-rust.sh ${1:-}"
fi

echo
vert "en-tetes prets dans third_party/gen-$CIBLE"
