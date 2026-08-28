#!/bin/bash
# Raccourci d'echeances : sauter un balayage ne saute jamais un reveil.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-echeances

echo "== echeances de reveil (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/smp/test_echeances.rs
"$SORTIE"
rm -f "$SORTIE"
