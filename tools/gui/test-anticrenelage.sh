#!/bin/bash
# Tests d'unite de l'anti-crenelage du compositeur.
#
# Ce fichier de tests existait sans script pour le lancer : il fallait le
# compiler a la main pour l'executer, ce qui revient a ne pas l'executer.
#
# Ce qu'il protege ne se voit sur aucune capture d'ecran : que la rampe reste
# FIDELE a la forme binaire -- pleine dedans, nulle dehors, bornee par elle,
# de meme aire, et surtout au meme endroit. Un anti-crenelage decale d'un
# pixel n'est pas un adoucissement, c'est un deplacement.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-anticrenelage

echo "== tests d'unite de l'anti-crenelage (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/gui/test_anticrenelage.rs
"$SORTIE"
rm -f "$SORTIE"
