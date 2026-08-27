#!/bin/bash
# Oracle d'equivalence de rendu (Gate 1C).
#
# Pour chaque rectangle de degat, le pipeline reel -- occlusion puis
# intersection -- doit produire exactement les memes pixels que le rendu de
# TOUS les calques avec le meme clip. Le tampon de depart contient la scene
# PRECEDENTE : c'est ainsi qu'un calque ecarte a tort laisse des pixels
# perimes, et c'est exactement ce qu'on veut detecter.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-rendu

echo "== oracle d'equivalence de rendu (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/gui/test_rendu.rs
"$SORTIE"
rm -f "$SORTIE"
