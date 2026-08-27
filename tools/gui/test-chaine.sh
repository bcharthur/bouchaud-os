#!/bin/bash
# Veilleur de la chaine entree -> degat -> trame -> present -> LFB.
#
# Deux exigences opposees : nommer le premier maillon rompu, et ne pas inonder
# la trace. Un diagnostic par mouvement de souris est aussi inutile qu'aucun.
#
# Code de retour : 0 si tout passe.

set -eu
cd "$(dirname "$0")/../.."

SORTIE=${TMPDIR:-/tmp}/bo-test-chaine

echo "== veilleur de la chaine entree -> LFB (hote) =="
rustc --edition 2021 --test -O -o "$SORTIE" tools/gui/test_chaine.rs
"$SORTIE"
rm -f "$SORTIE"
