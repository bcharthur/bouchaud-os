#!/bin/bash
# Systeme de fenetrage : geometrie, hit-test, politique de placement,
# rasterisation arrondie.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-fenetrage

echo "== systeme de fenetrage (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/gui/test_fenetrage.rs
"$SORTIE"
rm -f "$SORTIE"
