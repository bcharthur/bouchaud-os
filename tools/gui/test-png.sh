#!/bin/bash
# Decodeur PNG du noyau : les cinq filtres et les cinq types de couleur.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-png

echo "== decodeur PNG (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/gui/test_png.rs
"$SORTIE"
rm -f "$SORTIE"
