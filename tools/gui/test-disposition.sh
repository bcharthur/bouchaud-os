#!/bin/bash
# Tests d'unite de la geometrie du bureau.
#
# Deux barres de meme forme aux deux extremites de l'ecran, un menu et ses
# lignes. Confondre les deux barres ne se voit dans aucun compteur : le
# compositeur presente fidelement la zone qu'on lui designe, meme si les pixels
# qui ont change sont ailleurs.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-disposition

echo "== tests d'unite de la geometrie du bureau (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/gui/test_disposition.rs
"$SORTIE"
rm -f "$SORTIE"
